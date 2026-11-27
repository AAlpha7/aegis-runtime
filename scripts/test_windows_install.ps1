$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/install-windows.ps1" -LibraryOnly
$spec = Get-AegisTaskSpec 'Test-Distro' 'tester' '/home/tester/project with spaces' '0123456789abcdef' 'S-1-test'
if ($spec.LegacyArguments -notlike '*"/home/tester/project with spaces/scripts/managed-runtime.py"*') { throw 'Path was not quoted.' }
if (!$spec.LegacyArguments.StartsWith('--distribution Test-Distro --user tester --exec ')) { throw 'WSL distribution/user arguments must not gain literal quotes.' }
$task = [PSCustomObject]@{
    Description = $spec.Description
    Actions = @([PSCustomObject]@{Execute=$spec.Execute; Arguments=$spec.Arguments})
    Principal = [PSCustomObject]@{RunLevel='Limited'; UserId='S-1-test'; LogonType='Interactive'}
}
Assert-AegisTaskOwnership $task $spec 'C:\Windows\System32\wsl.exe' 'S-1-test'
foreach ($property in @('Description', 'Arguments', 'RunLevel', 'UserId', 'LogonType')) {
    $copy = $task | ConvertTo-Json -Depth 5 | ConvertFrom-Json
    switch ($property) {
        Description { $copy.Description = 'unrelated task' }
        Arguments { $copy.Actions[0].Arguments = '--shutdown' }
        RunLevel { $copy.Principal.RunLevel = 'Highest' }
        UserId { $copy.Principal.UserId = 'other-user' }
        LogonType { $copy.Principal.LogonType = 'Password' }
    }
    $rejected = $false
    try { Assert-AegisTaskOwnership $copy $spec 'C:\Windows\System32\wsl.exe' 'S-1-test' } catch { $rejected = $true }
    if (!$rejected) { throw "Ownership change accepted: $property" }
}
$rejected = $false
try { Get-AegisTaskSpec 'Test-Distro' 'tester' "/tmp/evil`nroot" '0123456789abcdef' 'S-1-test' } catch { $rejected = $true }
if (!$rejected) { throw 'Control character accepted.' }

# Execute the actual action bodies with a fake WSL command and browser launcher.
# No Task Scheduler writes, process restarts, browser windows or dialogs occur.
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    "$PSScriptRoot/install-windows.ps1", [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Installer has PowerShell syntax errors.' }
$dispatch = $ast.Find({ param($node)
    $node -is [System.Management.Automation.Language.SwitchStatementAst] -and $node.Condition.Extent.Text -eq '$Action'
}, $true)
function Test-LauncherAction {
    param([string]$Action, [bool]$Remote)
    $script:launchCalls = [System.Collections.Generic.List[string]]::new()
    function Start-ScheduledTask { param($TaskName) $script:launchCalls.Add('task') }
    function Start-Process { param($FilePath) $script:launchCalls.Add("browser:$FilePath") }
    $wslPath = {
        $operation = $args[-1]
        $script:launchCalls.Add("wsl:$operation")
        $global:LASTEXITCODE = 0
        if ($operation -eq 'login') { 'http://127.0.0.1:8765/auth/login/local?token=synthetic' }
    }
    $info = [PSCustomObject]@{remote=$Remote; browser_url='https://example.test/access'}
    $task = $true
    $spec = @{Name='synthetic'}
    $FromLauncher = $Remote # Remote login must not enter the restart dialog.
    $body = ($dispatch.Clauses | Where-Object { $_.Item1.Value -eq $Action }).Item2.Extent.Text
    & ([scriptblock]::Create($body.Substring(1, $body.Length - 2))) | Out-Null
    return ($script:launchCalls -join ',')
}
foreach ($action in @('Login', 'Open')) {
    $actual = Test-LauncherAction $action $true
    if ($actual -ne 'task,wsl:start,browser:https://example.test/access') {
        throw "Remote $action unexpectedly restarted or opened a different entry: $actual"
    }
}
$actual = Test-LauncherAction 'Login' $false
if ($actual -ne 'task,wsl:restart,wsl:login,browser:http://127.0.0.1:8765/auth/login/local?token=synthetic') {
    throw "Local login no longer refreshes its one-time entry: $actual"
}
Write-Output 'PASS: task ownership/path contracts and actual remote/local launcher action behavior. No real task was created.'

# Execute recovery-settings creation and an existing-task installation without
# writing Task Scheduler. This catches upgrades that leave old retry settings.
function New-ScheduledTaskSettingsSet {
    param([switch]$Hidden, [switch]$AllowStartIfOnBatteries, [switch]$DontStopIfGoingOnBatteries,
          $ExecutionTimeLimit, $MultipleInstances, [switch]$StartWhenAvailable,
          $RestartCount, $RestartInterval)
    return $PSBoundParameters
}
$recovery = New-AegisSessionSettings
if ($recovery.RestartCount -ne 999 -or $recovery.RestartInterval.TotalSeconds -ne 60 -or
    !$recovery.StartWhenAvailable -or $recovery.ExecutionTimeLimit -ne [TimeSpan]::Zero -or
    $recovery.MultipleInstances -ne 'IgnoreNew') { throw 'Recovery settings are incomplete.' }
$script:updatedSettings = $false
function Set-ScheduledTask { param($TaskName, $Settings, $Action) $script:updatedSettings = $Settings.RestartCount -eq 999 }
function Start-ScheduledTask { param($TaskName) }
function Stop-ScheduledTask { param($TaskName) }
function New-ScheduledTaskAction { param($Execute, $Argument) @{Execute=$Execute;Arguments=$Argument} }
function Start-Sleep { param($Seconds) }
function Get-ScheduledTask { param($TaskName) [PSCustomObject]@{State='Running'} }
function Set-AegisLaunchers { param($Spec, $Id) }
function Assert-AegisHostReady { param($Health, $Console) $script:readyChecked=$true }
$task = $true
$info = [PSCustomObject]@{id='0123456789abcdef'}
$body = ($dispatch.Clauses | Where-Object { $_.Item1.Value -eq 'Install' }).Item2.Extent.Text
& ([scriptblock]::Create($body.Substring(1, $body.Length - 2))) | Out-Null
if (!$script:updatedSettings -or !$script:readyChecked) { throw 'Reinstall did not update recovery settings and check actual readiness.' }
Write-Output 'PASS: existing-task recovery upgrade, minute retry and missed-start recovery.'

$legacy = $task = [PSCustomObject]@{
    Description=$spec.Description
    Actions=@([PSCustomObject]@{Execute='C:\Windows\System32\wsl.exe';Arguments=$spec.LegacyArguments})
    Principal=[PSCustomObject]@{RunLevel='Limited';UserId='S-1-test';LogonType='Interactive'}
}
Assert-AegisTaskOwnership $legacy $spec 'C:\Windows\System32\wsl.exe' 'S-1-test' -AllowLegacy
$legacy.Actions[0].Arguments += ' --unowned'
$rejected=$false
try { Assert-AegisTaskOwnership $legacy $spec 'C:\Windows\System32\wsl.exe' 'S-1-test' -AllowLegacy } catch { $rejected=$true }
if (!$rejected) { throw 'Legacy migration accepted changed command.' }
$script:attempts=0
$script:delays=0
try {
    Invoke-AegisSessionSupervisor -MaxAttempts 3 -RetrySeconds 1 -Run {
        $script:attempts++
        if ($script:attempts -eq 1) { throw 'transient launch failure' }
        # A clean return is also unexpected for a lifetime hold.
    } -Delay { param($Seconds) $script:delays++ }
} catch {
    if ($_.Exception.Message -notmatch 'recovery exhausted') { throw }
}
if ($script:attempts -ne 3 -or $script:delays -ne 2) { throw 'Supervisor failed to retry error/clean exits with bounded delay.' }
Write-Output 'PASS: session supervisor retries failed and clean exits; legacy action migration remains scoped.'
