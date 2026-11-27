# Windows half of a managed WSL installation. No Linux secrets are transferred.
[CmdletBinding()]
param(
    [ValidateSet('Install', 'Status', 'Open', 'Login', 'Verify', 'Remove')]
    [string]$Action = 'Install',
    [ValidatePattern('^[a-zA-Z0-9_.-]+$')][string]$Distribution,
    [ValidatePattern('^[a-zA-Z0-9_.-]+$')][string]$LinuxUser,
    [ValidatePattern('^/[^"\r\n]+$')][string]$LinuxRoot,
    [ValidateRange(10, 86400)][int]$Seconds = 120,
    [switch]$FromLauncher,
    [switch]$LibraryOnly
)
$ErrorActionPreference = 'Stop'
trap {
    if ($FromLauncher) {
        Add-Type -AssemblyName PresentationFramework
        [System.Windows.MessageBox]::Show("Aegis could not open. Ask your deployment assistant to check the service.`n`n$($_.Exception.Message)", 'Aegis') | Out-Null
    }
    break
}

function Invoke-AegisSessionSupervisor {
    param([scriptblock]$Run, [int]$MaxAttempts = 999, [int]$RetrySeconds = 60,
          [scriptblock]$Delay = { param($Seconds) Start-Sleep -Seconds $Seconds })
    # The Linux hold should never return. Retry both failed and unexpected clean
    # exits, including transient WSL startup errors, without a deployment agent.
    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        try { & $Run | Out-Null } catch { }
        if ($attempt -lt $MaxAttempts) { & $Delay $RetrySeconds }
    }
    throw 'Aegis session recovery exhausted. Check the WSL installation before restarting the task.'
}

function Get-AegisTaskSpec {
    param([string]$Distro, [string]$User, [string]$Root, [string]$Id, [string]$Sid)
    if ($Distro -notmatch '^[a-zA-Z0-9_.-]+$' -or $User -notmatch '^[a-zA-Z0-9_.-]+$' -or
        $Root -notmatch '^/[^"\r\n]+$' -or $Id -notmatch '^[a-f0-9]{16}$') {
        throw 'Invalid installation identity or path.'
    }
    $wsl = Join-Path $env:SystemRoot 'System32\wsl.exe'
    $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $quotedRoot = $Root.Replace("'", "''")
    $quotedWsl = $wsl.Replace("'", "''")
    $supervisor = ${function:Invoke-AegisSessionSupervisor}.ToString()
    $command = "function Invoke-AegisSessionSupervisor { $supervisor }; Invoke-AegisSessionSupervisor -Run { & '$quotedWsl' --distribution '$Distro' --user '$User' --exec python3 '$quotedRoot/scripts/managed-runtime.py' --root '$quotedRoot' hold }"
    @{
        Name = "AegisRuntime-$Distro-$Id"
        Description = "aegis-runtime-managed-v1|$Sid|$Distro|$User|$Root"
        Execute = $powershell
        Arguments = '-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -EncodedCommand ' + [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
        LegacyArguments = "--distribution $Distro --user $User --exec python3 `"$Root/scripts/managed-runtime.py`" --root `"$Root`" hold"
    }
}

function Assert-AegisTaskOwnership {
    param($Task, $Spec, [string]$WslPath, [string]$Sid, [switch]$AllowLegacy)
    $principalSid = if ($Task) { [string]$Task.Principal.UserId } else { '' }
    # Task Scheduler canonicalizes a supplied SID to a local account name.
    if ($Task -and $principalSid -notmatch '^S-1-') {
        try { $principalSid = ([Security.Principal.NTAccount]::new($principalSid)).Translate([Security.Principal.SecurityIdentifier]).Value }
        catch { throw 'Task principal cannot be resolved; nothing changed.' }
    }
    $ownedAction = $Task -and @($Task.Actions).Count -eq 1 -and (
        ($Task.Actions[0].Execute -ieq $Spec.Execute -and $Task.Actions[0].Arguments -ceq $Spec.Arguments) -or
        ($AllowLegacy -and $Task.Actions[0].Execute -ieq $WslPath -and $Task.Actions[0].Arguments -ceq $Spec.LegacyArguments))
    if ($Task -and ($Task.Description -cne $Spec.Description -or
        !$ownedAction -or $Task.Principal.RunLevel -ne 'Limited' -or
        $principalSid -ne $Sid -or [string]$Task.Principal.LogonType -ne 'Interactive')) {
        throw 'Task ownership or action changed. Nothing overwritten or removed.'
    }
}

function New-AegisSessionSettings {
    # A transient WSL failure may outlast the old three-minute retry budget.
    # Task Scheduler retries failed exits while this user is logged in, without
    # waking the computer or depending on a deployment-agent process.
    New-ScheduledTaskSettingsSet -Hidden -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -StartWhenAvailable `
        -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1)
}

function Test-AegisHostIdle {
    param([string]$TaskName, [string]$Health, [string]$Console, [int]$Duration)
    foreach ($url in @($Health, $Console)) {
        if ($url -notmatch '^http://127\.0\.0\.1:[0-9]+/(health|ui/)$') { throw 'Expected loopback verification URL.' }
    }
    # IMPORTANT: no WSL command, UNC access, or Linux probe in this interval.
    # Such probes could wake WSL and turn the original failure into a false pass.
    $until = [DateTime]::UtcNow.AddSeconds($Duration)
    do {
        if ((Get-ScheduledTask -TaskName $TaskName).State -ne 'Running') { throw 'Windows session task is not running.' }
        foreach ($url in @($Health, $Console)) {
            try { $response = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 5 }
            catch { throw "Windows cannot reach $url. Check the Linux service, WSL localhost forwarding and Windows port conflicts/reserved ranges. Verification failed; no alternate port or network exposure was selected." }
            if ($response.StatusCode -ne 200) { throw 'Host-side HTTP check failed.' }
        }
        Start-Sleep -Seconds 5
    } while ([DateTime]::UtcNow -lt $until)
}

function Assert-AegisHostReady {
    param([string]$Health, [string]$Console)
    foreach ($url in @($Health, $Console)) {
        if ($url -notmatch '^http://127\.0\.0\.1:[0-9]+/(health|ui/)$') { throw 'Expected loopback readiness URL.' }
        try { $response = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 5 }
        catch { throw 'The Windows session supervisor is running, but Aegis is not reachable yet. Setup is incomplete; check the service and retry.' }
        if ($response.StatusCode -ne 200) { throw 'Aegis readiness failed; setup is incomplete.' }
    }
}

function Set-AegisLaunchers {
    param($Spec, [string]$Id, [switch]$Remove)
    $programs = [Environment]::GetFolderPath('Programs')
    $folder = Join-Path (Join-Path $programs 'Aegis Runtime') $Id
    # Only exact owned shortcut files are ever removed; no recursive deletion.
    if (![IO.Path]::GetFullPath($folder).StartsWith([IO.Path]::GetFullPath($programs) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Launcher directory escaped the current-user Programs folder.'
    }
    $shell = New-Object -ComObject WScript.Shell
    $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    foreach ($entry in @(@('Aegis', 'Open'), @('Sign in to Aegis', 'Login'))) {
        $path = Join-Path $folder ($entry[0] + " ($Distribution).lnk")
        $arguments = "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$PSCommandPath`" -FromLauncher -Action $($entry[1]) -Distribution $Distribution -LinuxUser $LinuxUser -LinuxRoot `"$LinuxRoot`""
        $shortcut = $shell.CreateShortcut($path)
        if ((Test-Path -LiteralPath $path) -and ($shortcut.Description -cne $Spec.Description -or
            $shortcut.TargetPath -ine $powershell -or $shortcut.Arguments -cne $arguments)) {
            throw 'An Aegis launcher was changed externally; it was preserved.'
        }
        if ($Remove) {
            if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
        } else {
            New-Item -ItemType Directory -Path $folder -Force | Out-Null
            $shortcut.TargetPath = $powershell
            $shortcut.Arguments = $arguments
            $shortcut.Description = $Spec.Description
            $shortcut.WindowStyle = 7
            $shortcut.Save()
        }
    }
    if ($Remove -and (Test-Path -LiteralPath $folder) -and !(Get-ChildItem -LiteralPath $folder -Force)) {
        Remove-Item -LiteralPath $folder
    }
}

if ($LibraryOnly) { return }
if (!$Distribution -or !$LinuxUser -or !$LinuxRoot) {
    throw 'Supply -Distribution, -LinuxUser and -LinuxRoot for the installation inside WSL. See docs/INSTALLATION.md.'
}
$wslPath = Join-Path $env:SystemRoot 'System32\wsl.exe'
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$helper = "$LinuxRoot/scripts/managed-runtime.py"
if ($Action -eq 'Remove') {
    # Removal still works after the Linux service/checkout has been removed.
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $id = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($LinuxRoot))).Replace('-', '').ToLowerInvariant()).Substring(0, 16) }
    finally { $sha.Dispose() }
    $info = [PSCustomObject]@{ id = $id; root = $LinuxRoot }
} else {
    $raw = & $wslPath --distribution $Distribution --user $LinuxUser --exec python3 $helper --root $LinuxRoot windows-info
    if ($LASTEXITCODE -ne 0) { throw 'Complete the Linux managed installation first. No Windows task was changed.' }
    $info = $raw | ConvertFrom-Json
    if ($info.root -cne $LinuxRoot) { throw 'Use the canonical Linux checkout path reported by the installer.' }
}
$spec = Get-AegisTaskSpec $Distribution $LinuxUser $LinuxRoot $info.id $sid
$task = Get-ScheduledTask -TaskName $spec.Name -ErrorAction SilentlyContinue
Assert-AegisTaskOwnership $task $spec $wslPath $sid -AllowLegacy

switch ($Action) {
    'Install' {
        $settings = New-AegisSessionSettings
        $taskAction = New-ScheduledTaskAction -Execute $spec.Execute -Argument $spec.Arguments
        if (!$task) {
            $trigger = New-ScheduledTaskTrigger -AtLogOn -User $sid
            $principal = New-ScheduledTaskPrincipal -UserId $sid -LogonType Interactive -RunLevel Limited
            Register-ScheduledTask -TaskName $spec.Name -Action $taskAction -Trigger $trigger `
                -Principal $principal -Settings $settings -Description $spec.Description | Out-Null
        } else {
            # Migrate only the exactly recognized old action. Preserve identity,
            # principal and triggers; restart the host hold, not the Linux service.
            Stop-ScheduledTask -TaskName $spec.Name
            Set-ScheduledTask -TaskName $spec.Name -Action $taskAction -Settings $settings | Out-Null
        }
        Start-ScheduledTask -TaskName $spec.Name
        Start-Sleep -Seconds 3
        if ((Get-ScheduledTask -TaskName $spec.Name).State -ne 'Running') {
            throw 'The task was created but did not stay running. Check the distribution/user/path and Task Scheduler result, then retry. Deployment is not complete.'
        }
        Assert-AegisHostReady $info.health $info.console
        Set-AegisLaunchers $spec $info.id
        Write-Output "Installed current-user task: $($spec.Name). No administrator grant or password was stored."
        Write-Output 'WSL stays available while this Windows user is logged in. Sleep, logout and shutdown interrupt access.'
        Write-Output 'The Windows session supervisor retries disconnected WSL holds every minute, up to 999 attempts. Persistent failures still require repair; this does not keep the computer awake.'
        Write-Output 'Start menu entries added: Aegis and Sign in to Aegis. Local sign-in confirms a restart for a fresh link; remote sign-in opens Passkeys without restarting. No global execution policy was changed.'
        Write-Output 'Close the deployment agent and its terminals, then run this script with -Action Verify -Seconds 1800 from a separate Windows PowerShell.'
    }
    'Status' {
        if (!$task) { throw 'Windows task is not installed.' }
        Write-Output "Task: $($task.State). Console: $($info.browser_url)"
    }
    'Open' {
        if (!$task) { throw 'Windows task is not installed.' }
        Start-ScheduledTask -TaskName $spec.Name
        & $wslPath --distribution $Distribution --user $LinuxUser --exec python3 $helper --root $LinuxRoot start
        if ($LASTEXITCODE -ne 0) { throw 'Aegis did not become ready.' }
        Start-Process $info.browser_url
        if ($info.remote) { Write-Output 'Sign in with your Passkey. Private access requires Tailscale on this device.' }
        else { Write-Output 'If login is needed, run this launcher with -Action Login. It restarts Aegis to obtain a fresh one-time link.' }
    }
    'Login' {
        if (!$task) { throw 'Windows task is not installed.' }
        if ($info.remote) {
            Start-ScheduledTask -TaskName $spec.Name
            & $wslPath --distribution $Distribution --user $LinuxUser --exec python3 $helper --root $LinuxRoot start
            if ($LASTEXITCODE -ne 0) { throw 'Service start failed. No browser was opened.' }
            Write-Output 'Opening Passkey sign-in without restarting. Private access requires Tailscale on this device.'
            Start-Process $info.browser_url
            return
        }
        if ($FromLauncher) {
            Add-Type -AssemblyName PresentationFramework
            $choice = [System.Windows.MessageBox]::Show(
                'Aegis will restart to create a fresh private sign-in link. Active requests may be interrupted. Continue?',
                'Sign in to Aegis', 'OKCancel', 'Information')
            if ($choice -ne 'OK') { return }
        }
        Write-Output 'Restarting Aegis to create a fresh private login link. Active requests may be interrupted.'
        Start-ScheduledTask -TaskName $spec.Name
        & $wslPath --distribution $Distribution --user $LinuxUser --exec python3 $helper --root $LinuxRoot restart
        if ($LASTEXITCODE -ne 0) { throw 'Service restart failed. No browser was opened.' }
        $link = & $wslPath --distribution $Distribution --user $LinuxUser --exec python3 $helper --root $LinuxRoot login 2>$null
        if ($LASTEXITCODE -ne 0 -or $link -notmatch '^https?://') { throw 'No login entry is available.' }
        # Keep the private link out of the console/history; open it directly.
        Start-Process ([string]$link).Trim()
    }
    'Verify' {
        if (!$task) { throw 'Windows task is not installed.' }
        $before = (& $wslPath --distribution $Distribution --user $LinuxUser --exec cat /proc/sys/kernel/random/boot_id).Trim()
        if ($LASTEXITCODE -ne 0 -or !$before) { throw 'Cannot read initial boot identity.' }
        Write-Output "Checking Windows HTTP access for $Seconds seconds. Keep the deployment agent closed and do not run Linux commands."
        Test-AegisHostIdle $spec.Name $info.health $info.console $Seconds
        $after = (& $wslPath --distribution $Distribution --user $LinuxUser --exec cat /proc/sys/kernel/random/boot_id).Trim()
        if ($LASTEXITCODE -ne 0 -or $before -ne $after) { throw 'WSL rebooted during verification.' }
        Write-Output "PASS: current-user task running, same WSL boot, host HTTP 200 for $Seconds seconds. This does not test Windows reboot, logout/login or mobile access."
    }
    'Remove' {
        Set-AegisLaunchers $spec $info.id -Remove
        if ($task) {
            Stop-ScheduledTask -TaskName $spec.Name
            Unregister-ScheduledTask -TaskName $spec.Name -Confirm:$false
        }
        Write-Output 'Removed only this Windows session task. Linux services, data and key are retained.'
    }
}
