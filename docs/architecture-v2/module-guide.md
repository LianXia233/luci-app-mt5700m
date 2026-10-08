# 模块指南：新增一个功能

一个功能就是一个模块。它绝不是前端里的一个新解析器、`daemon.rs` 里的
一个新分支，也不是某个已有解码器的第二份拷贝。

## 1. 目录布局

```text
src/modules/<name>/
├── mod.rs        one paragraph: what this module owns
├── commands.rs   AT command constants/builders
├── state.rs      typed domain model + to_json()/from_json() (+ tests)
├── parser.rs     AT text -> typed state (+ tests, one per response variant)
├── service.rs    refresh policy + spawn() of periodic jobs
└── api.rs        routes() + text renderers for the CLI
```

## 2. 步骤

1. **命令** —— 把该领域需要的每一条 AT 字面量放进 `commands.rs`。
   模块内其他地方不得出现任何 AT 字符串。
2. **解析器** —— 把响应解码成 `state.rs` 里的结构体。测试放在
   `parser.rs`（每种变体一个：正常路径、厂商变体、错误/缺失）。
3. **状态** —— `to_json()` 就是线上契约；字段名沿用已有
   消费方正在读取的那些。如果 CLI 必须打印文本，就在这里加 `to_text()`，
   这样 JSON 与文本永远不会互相矛盾。
4. **服务** —— 用下面的方式读取：

   ```rust
   ctx.read(cmd, at_timeout, queued_timeout, Priority::Low)?  // background
   ctx.slow(key, cmd, at_timeout, queued_timeout, backoff)     // backoff-gated
   ctx.query(cmd)?                                            // interactive read
   ctx.action(cmd)?                                           // write
   ```

   然后用 `ctx.store(TOPIC, EVENT, &state.to_json())` 发布，或用
   `ctx.stale(TOPIC, EVENT)` 回退。绝不触碰 `AtArbiter`、tty 或套接字。
5. **Spawn** —— 从 `spawn(tasks)` 里调用 `run_in_task`；在
   `modules::spawn_all` 中注册该任务。挑选与该主题的
   新鲜度承诺相匹配的周期/TTL 组合。
6. **API** —— 添加 `Route` 并在 `api/registry.rs::routes()` 中注册。
7. **接线** —— `mod.rs`（文档）、`modules/mod.rs`（模块 + spawn）、registry
   （路由）。这就是全部的注册面。

## 3. 提交前检查清单

* `commands.rs` 之外没有 AT 字面量；模块内没有 `crate::serial`、
  `crate::scheduler::arbiter` 或 `std::net`/`std::fs` 的 tty 访问。
* 每一种新的响应形状都有一个单元测试，样本是现场抓取的真实
  调制解调器应答（含被截断的/异常的变体）。
* `python3 scripts/rs-static-check.py mt5700webui-openwrt-server/at-webserver/src`
  （模块树、`crate::` 路径、调用元数、局部调用、trait 实现、结构体
  字段、枚举变体、堆叠 derive、未导入的模块限定调用，以及同一个局部变量上
  重叠的闭包）与 `cargo test` 均为干净；两者都在 CI 中运行，且该
  检查器自身的检查也经过变异测试（见 `migration.md` 的「Errors &
  dead ends」清单）。
* 前端 diff 中**不**含 AT 字符串，也不含新的业务规则。
