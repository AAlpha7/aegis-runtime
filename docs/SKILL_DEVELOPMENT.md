# Aegis Runtime Skill 开发指南

Status: active skill authoring guide for hand-written Wasm skills under
`skills/`. Autonomous SkillForge generation was removed (see
`docs/2026-09-04-feature-removal-plan.md`).

本文档介绍如何为 Aegis Runtime 创建新的 WebAssembly (Wasm) 技能插件。

---

## 概述

Aegis Runtime 技能是编译为 `wasm32-wasip1` 的独立 Rust crate。它们在沙箱中运行，通过宿主函数与外部世界交互，所有网络访问受域名白名单严格限制。

```
用户消息 → LLM 选择 Skill → IntentEnvelope 签名
    → Wasm 沙箱实例化 → 执行 run() → 结果返回 → 实例销毁
```

## 快速创建一个新 Skill

### 1. 创建 crate 目录

```bash
mkdir -p skills/my_skill/src
```

### 2. 编写 Cargo.toml

```toml
# skills/my_skill/Cargo.toml
[lints]
workspace = true

[package]
name = "my-skill"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
# 可选但推荐：共享的宿主 FFI 声明 + payload/输出辅助函数（仓库内所有技能都用它）
aegis-runtime-wasm-core = { path = "../aegis_runtime_wasm_core" }
```

**关键点：** `crate-type = ["cdylib"]` 是必须的，它告诉编译器生成 Wasm 动态库。`aegis-runtime-wasm-core`（`skills/aegis_runtime_wasm_core/`）封装了下文的宿主函数声明并提供 `read_payload()` / `write_json()` / `fetch_url()` / `ProbeSignal` 等辅助；最短的参考实现见 `skills/test_no_net/src/lib.rs`。下文第 4 步展示的是不依赖它的裸 ABI 写法。

### 3. 编写 manifest.json

```json
{
  "skill_name": "my_skill",
  "max_memory_mb": 2,
  "allowed_domains": ["api.example.com"],
  "allowed_fs_paths": [],
  "cron_schedule": null,
  "description": "简要描述这个 Skill 的功能。LLM 根据此描述决定何时调用。",
  "payload_schema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "The search query"
      }
    },
    "required": ["query"]
  },
  "menu_command": "mysearch",
  "menu_description": "Search via MySkill"
}
```

**字段说明：**

| 字段 | 类型 | 说明 |
|---|---|---|
| `skill_name` | string | 必须与目录名一致 |
| `max_memory_mb` | u32 | Wasm 实例内存上限（默认 2 MB） |
| `allowed_domains` | string[] | 网络访问白名单。`["*"]` 表示不限制，`[]` 表示禁止网络 |
| `allowed_fs_paths` | string[] | 文件读取白名单目录，供 `host_read_file` / `host_list_files` 使用；`[]` 表示禁止读文件 |
| `cron_schedule` | string? | Cron 表达式，设置后 Chronos 自动定期执行。`null` 表示仅按需调用 |
| `description` | string | LLM 用来决定何时调用此 Skill 的描述 |
| `payload_schema` | object | JSON Schema，定义 LLM 应传递的参数 |
| `menu_command` | string? | 激活时注册的 Telegram Bot 菜单命令。小写字母/数字/下划线，1-32 字符 |
| `menu_description` | string? | 菜单命令旁显示的描述文字，最长 256 字符 |

### 4. 编写 Skill 逻辑

```rust
// skills/my_skill/src/lib.rs
#![allow(unsafe_code)] // edition 2024 requires unsafe(no_mangle) on WASM entry points

use std::cell::RefCell;

// ── 宿主函数声明（由 Aegis Runtime 沙箱注入；edition 2024 要求 `unsafe extern`） ──
#[link(wasm_import_module = "aegis")]
unsafe extern "C" {
    /// 发起 HTTP GET 请求。返回值 >= 0 表示成功，-1 表示域名被白名单拦截。
    fn host_fetch(url_ptr: i32, url_len: i32) -> i32;

    /// 将最终结果写回宿主。
    fn host_write_output(ptr: i32, len: i32);

    /// 获取宿主传入的 payload 长度。
    fn host_get_payload_len() -> i32;

    /// 将 payload 数据复制到指定内存地址。
    fn host_get_payload(dest_ptr: i32);

    /// 获取最近一次 host_fetch 响应体的长度。
    fn host_get_fetch_response_len() -> i32;

    /// 将响应体数据复制到指定内存地址。返回 0 成功，-1 越界。
    fn host_get_fetch_response(dest_ptr: i32) -> i32;
}

// ── 数据结构 ────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct Payload {
    query: String,
}

#[derive(serde::Serialize)]
struct Output {
    status: &'static str,
    result: String,
}

// ── 输出缓冲区（防止被 Wasm GC 回收） ──────────────────

thread_local! {
    static OUTPUT_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn write_output(output: &impl serde::Serialize) {
    let bytes = serde_json::to_vec(output).unwrap_or_default();
    OUTPUT_BUF.with(|buf| {
        *buf.borrow_mut() = bytes;
        let b = buf.borrow();
        unsafe { host_write_output(b.as_ptr() as i32, b.len() as i32) };
    });
}

// ── 入口函数（必须导出） ────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    // 1. 读取 payload
    let payload_len = unsafe { host_get_payload_len() } as usize;
    let mut payload_buf = vec![0u8; payload_len];
    unsafe { host_get_payload(payload_buf.as_mut_ptr() as i32) };

    // 2. 解析 payload
    let payload: Payload = match serde_json::from_slice(&payload_buf) {
        Ok(p) => p,
        Err(e) => {
            write_output(&Output {
                status: "error",
                result: format!("invalid payload: {e}"),
            });
            return;
        }
    };

    // 3. 调用外部 API（受白名单保护）
    let url = format!("https://api.example.com/search?q={}", payload.query);
    let url_bytes = url.as_bytes();
    let code = unsafe { host_fetch(url_bytes.as_ptr() as i32, url_bytes.len() as i32) };

    if code < 0 {
        write_output(&Output {
            status: "blocked",
            result: "Domain not in whitelist".to_string(),
        });
        return;
    }

    // 4. 读取响应
    let resp_len = unsafe { host_get_fetch_response_len() } as usize;
    let mut resp_buf = vec![0u8; resp_len];
    unsafe { host_get_fetch_response(resp_buf.as_mut_ptr() as i32) };
    let body = String::from_utf8_lossy(&resp_buf).to_string();

    // 5. 返回结果
    write_output(&Output {
        status: "ok",
        result: body,
    });
}

// ── 内存分配器（宿主用来在 Wasm 内存中写入数据） ────────

#[unsafe(no_mangle)]
pub extern "C" fn alloc(size: i32) -> i32 {
    let layout = std::alloc::Layout::from_size_align(size as usize, 1)
        .expect("invalid alloc layout");
    unsafe { std::alloc::alloc(layout) as i32 }
}
```

### 5. 注册到 workspace

编辑根目录 `Cargo.toml`，将新 Skill 加入 workspace：

```toml
[workspace]
members = ["core", "skills/aegis_runtime_wasm_core", "skills/web_reader", "skills/server_monitor", "skills/my_skill"]
resolver = "2"
```

### 6. 编译和测试

```bash
# 方式一：core/build.rs 会把 skills/ 下的每个 crate 编译成 wasm32-wasip1 并复制到 compiled_skills/
cargo build --release
# （只改 core 时可设 SKIP_SKILL_BUILD=1 跳过技能编译；SKILL_BUILD_ONLY=my_skill 只编某一个）

# 方式二：单独编译一个技能（不经 build.rs）
cargo build -p my-skill --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/my_skill.wasm compiled_skills/

# 只做类型检查（全部技能 crate，wasm32-wasip1 目标；CI 也跑这个）
make skills-check

# 验证 Wasm 产物
ls compiled_skills/
# 应看到：my_skill.wasm

# 运行测试（不要用 cargo test --workspace：工作区含 wasm32 技能 crate，宿主目标编译不过）
make test
```

> `compiled_skills/*.wasm` 不入库；`core/tests/structural_guards.rs` 与 `core/tests/wasm_skill_completeness.rs` 会检查编译产物数量与可加载性，新增技能后需同步守卫常量。

## 宿主函数 API 参考

| 函数 | 参数 | 返回值 | 说明 |
|---|---|---|---|
| `host_fetch(url_ptr, url_len)` | URL 的内存指针和长度 | `>= 0` 成功 / `-1` 域名被拦截 | 发起 HTTP GET，受 `allowed_domains` 限制 |
| `host_write_output(ptr, len)` | 输出数据的指针和长度 | 无 | 将 JSON 结果写回宿主（见下方语义说明） |
| `host_get_payload_len()` | 无 | payload 字节长度 | 获取输入 payload 大小 |
| `host_get_payload(dest_ptr)` | 目标内存地址 | 无 | 将 payload 复制到 Wasm 内存 |
| `host_get_fetch_response_len()` | 无 | 响应体字节长度 | 获取最近一次 fetch 的响应大小 |
| `host_get_fetch_response(dest_ptr)` | 目标内存地址 | `0` 成功 / `-1` 越界 | 将响应体复制到 Wasm 内存 |
| `host_read_file(path_ptr, path_len)` | 路径指针和长度 | `>= 0` 成功 / 负数失败 | 读取 `allowed_fs_paths` 白名单内的文件（结果经 `host_get_fetch_response*` 读取） |
| `host_list_files()` | 无 | `>= 0` 成功 / 负数失败 | 列出 `allowed_fs_paths` 目录内的文件（symlink 经 `canonicalize()` 校验） |
| `host_store_state(key_ptr, key_len, val_ptr, val_len)` | 键/值指针和长度 | `0` 成功 / 负数失败 | 按技能持久化一段状态（跨调用保留） |
| `host_load_state(key_ptr, key_len)` | 键指针和长度 | 状态字节长度 / 负数失败 | 读取持久化状态；随后用 `host_get_state_response_len()` / `host_get_state_response(dest_ptr)` 复制 |

所有函数通过 `#[link(wasm_import_module = "aegis")]` 导入；完整声明以 `skills/aegis_runtime_wasm_core/src/lib.rs` 与宿主侧 `core/src/sandbox_host_fns.rs` 为准。

### `host_write_output` 语义

`host_write_output` 采用**最后一次调用生效**（last-call-wins）语义：

- 每次调用会**覆盖**之前的输出缓冲区，不会追加。
- 宿主在 `run()` 返回后读取缓冲区内容作为 Skill 的最终输出。
- 如果从未调用 `host_write_output`，Skill 输出为空字符串。
- 推荐模式：构建完整的 JSON 响应后**调用一次**。如果有多个出口（错误分支 / 正常分支），确保每个分支都调用。

## 必须导出的函数

| 导出名 | 签名 | 说明 |
|---|---|---|
| `run` | `extern "C" fn()` | Skill 入口点，每次调用时执行 |
| `alloc` | `extern "C" fn(i32) -> i32` | 内存分配器，宿主用来在 Wasm 内存中写入 payload |

## 创建心跳 Skill（Cron 定时任务）

如果 Skill 需要定期执行（如服务器健康监控，参考 `skills/server_monitor`），在 `manifest.json` 中设置 `cron_schedule`：

```json
{
  "cron_schedule": "0 */5 * * * *"
}
```

Cron 表达式格式：`秒 分 时 日 月 周`

**最小间隔限制：** 设计验证阶段会拒绝触发频率高于每 60 秒一次的 Cron 表达式。

| 表达式 | 含义 | 是否合法 |
|--------|------|----------|
| `* * * * * *` | 每秒 | 拒绝 |
| `*/5 * * * * *` | 每 5 秒 | 拒绝 |
| `0 * * * * *` | 每分钟 | 合法 |
| `0 */5 * * * *` | 每 5 分钟 | 合法 |

心跳 Skill 的 `run()` 函数应返回 `ProbeSignal` 格式的 JSON：

```rust
#[derive(serde::Serialize)]
struct ProbeSignal {
    require_llm_attention: bool,   // true = 触发 LLM 润色 + Telegram 推送
    event_payload: Option<String>, // 警报内容（仅 true 时需要）
}
```

- `require_llm_attention: false` → 静默结束，0 Token 消耗，0 推送
- `require_llm_attention: true` → Chronos 引擎拉起 LLM 润色并推送给 owner（已配置 Telegram 时推送到 Telegram）

使用 `aegis-runtime-wasm-core` 时可直接返回 `core::ProbeSignal`（`ProbeSignal::quiet()` 表示静默）。

## 技能运行时管理

内置技能通过 `skill_proposals` 注册（启动时从 `compiled_skills/` 自动登记）。技能源码在 `skills/` 下手工维护并编译；运行时不再提供自然语言自动创建或 `fix_skill` 修复管线。

| 伪工具 | 作用 |
|--------|------|
| `manage_skill` | `health`（成功率/延迟/错误分类）、`affected_intents`（引用该技能的 recurring intents） |
| `deactivate_skill` | 停用技能（保留产物，可再激活） |
| `reactivate_skill` | 重新激活已停用技能 |

## 分层权限模型

每个技能受三层权限控制（每层只能收紧，不能放宽）：

1. **全局策略**（硬编码默认值：无文件系统访问，仅 fetch 域名白名单）
2. **技能策略**（在 `manifest.json` / `SkillCapability` 中声明）
3. **执行时策略**（编排器根据上下文进一步限制）

```json
{
  "allowed_domains": ["api.example.com"],
  "allowed_fs_paths": [],
  "max_memory_mb": 2
}
```

## 安全约束

- Wasm 实例**每次调用后立即销毁**，无状态残留
- 网络访问**仅限 `allowed_domains` 白名单**内的域名
- 内存上限由 `max_memory_mb` 控制，超限触发 Trap
- **禁止** `std::process::Command` 或任何系统调用
- 所有调用必须经过 Ed25519 签名的 IntentEnvelope
- 提示注入防护：用户输入经 XML 标签剥离 + 隔离标签包裹后注入 LLM 提示词
- **SSRF 防护**：私有 IP 拦截覆盖 RFC 1918 + `100.64.0.0/10`（CGN/云内网）+ 链路本地 + IPv6 映射地址。白名单域名同样进行 DNS 解析后 IP 地址检查，防止域名解析到内网 IP

## 调试与输出处理

### 运行时日志调试

技能执行相关日志可在运行时对助理说"把日志调成 debug"（Web 控制台或 Telegram，`set_log_level` 伪工具）实时开启，无需重启。常用指令：

```
info,aegis_runtime_core::sandbox=debug    # Wasm 执行详情
info,aegis_runtime_core::orchestrator=debug  # 工具调用链路
```

### 长输出处理

技能返回大文本时，系统自动处理：
- 回复分片（4096 字节/片）+ HTML 标签感知拆分
- 超过 3 片时自动以 `.txt` 文档形式发送，避免消息刷屏
- 技能无需特殊处理，编排器透明管理

## 保留伪工具名称

以下名称是系统伪工具，**不可用作 Wasm 技能名称**（加载时会被遮蔽）：

| 伪工具 | 功能 |
|--------|------|
| `web_search` | OpenRouter Grok:online 搜索 |
| `browser_fetch` | 本机 Chrome 读网页 / 截图 |
| `run_external` | 委派给外部执行器（grok build / codex），Google 服务走这里 |
| `render_webapp` | SDUI 动态面板渲染 |
| `schedule_intent` | 延迟意图（提醒/回调）— recurring 需 group_key 去重 + 可选 max_fires 自动停止 |
| `list_intents` | 列出当前用户 pending intents |
| `cancel_intent` | 按 ID 取消 intent |
| `manage_skill` | 技能健康检查 / affected intents |
| `deactivate_skill` / `reactivate_skill` | 停用或重新激活技能 |

完整伪工具集合以 `core/src/tool_schemas.rs` 为准；上表列出技能作者最常碰到的保留名。

## v2 Component Model 技能（WIT 接口）

> **⚠️ v2 Component-Model 管道当前未端到端接通。** 所有线上技能均走 v1
> `extern "C"` 手动 ABI（`aegis_runtime_wasm_core` 提供 host FFI）。下方章节保留
> 是为了未来迁移参考，**切勿用于新技能开发**。

v2 技能使用 WebAssembly Component Model + `wit-bindgen`，提供编译时类型安全的接口。

### 与 v1 的区别

| 项目 | v1 (Core Module) | v2 (Component) |
|------|-----------------|----------------|
| 编译目标 | `wasm32-wasip1` | `wasm32-unknown-unknown` + `wasm-tools component new` |
| Host 函数 | `extern "C"` + 指针传递 | `wit-bindgen` 生成，typed signatures |
| Output | `host_write_output(ptr, len)` | `fn run(payload: String) -> String` |
| ProbeSignal | 手动 JSON 序列化，字段名靠约定 | WIT record，字段名编译时锁定 |
| alloc | 需要手写 `alloc()` 导出 | 不需要 |
| 向后兼容 | — | v1 技能继续正常工作 |

### v2 Probe 技能示例

```rust
wit_bindgen::generate!({
    path: "wit",
    world: "aegis-probe-v2",
});

struct MyProbe;

impl exports::aegis::skill::probe_v2::Guest for MyProbe {
    fn run(payload_json: String) -> aegis::skill::types::ProbeSignal {
        let body = match aegis::skill::host_v2::fetch("https://api.example.com/health") {
            Ok(text) => text,
            Err(_) => return aegis::skill::types::ProbeSignal {
                require_llm_attention: true,
                event_payload: Some("Health check failed".to_string()),
            },
        };

        aegis::skill::types::ProbeSignal {
            require_llm_attention: false,
            event_payload: None,
        }
    }
}

export!(MyProbe);
```

### SkillCapability 标记

```json
{
  "component_model": true,
  "skill_pattern": "probe_heartbeat"
}
```

设置 `component_model: true` 后，SandboxEngine 自动以 Component 模式加载和执行。
