# MT5700M 架构 v2

一个后端，两个彼此独立的前端：

```text
LuCI ──rpc/ucode──┐
                  ├──▶ Unified Rust backend (at-webserver) ──▶ AT scheduler ──▶ serial
WebUI ──WebSocket─┘
```

后端拥有调制解调器。前端只拥有像素：它们渲染领域 JSON、
发送具名的 API 调用，并只保留 UI 局部状态（当前标签页、弹窗、表单
内容、加载标志）。

## 文档

| 文件 | 内容 |
| ---- | -------- |
| [architecture.md](architecture.md) | 分层、模块树、依赖规则，以及谁可以依赖谁 |
| [data-flow.md](data-flow.md) | AT → Parser → Domain → Cache → API → UI 流水线、事件总线、任务生命周期、单一 AT 所有者时序 |
| [api-contract.md](api-contract.md) | 路由表、各传输层的信封、兼容性保证（CLI/ucode/WebSocket） |
| [module-guide.md](module-guide.md) | 配方：把一个功能作为模块加入（State/Commands/Parser/Service/API） |
| [migration.md](migration.md) | 本次重构迁走了什么、什么已被验证、还剩什么 |

## 五条规则

1. **唯一 AT 所有者。** 守护进程的 `serial::manager` 持有 tty（TIOCEXCL）。
   其他所有进程都通过控制套接字访问调制解调器。
2. **每种能力只有一份实现。** 解析器、缓存、API 处理器或
   重试策略都只存在一份，位于拥有该领域的模块内。
3. **模块之间通过接口通信，而不是内部实现。** AT 用 `AtChannel`，
   缓存 + 总线用 `RefreshCtx`，API 用 `ApiCtx`/`Route`，跨模块数据用
   已缓存主题。任何模块都不得触碰串口、仲裁器、另一个
   模块的私有成员或某种前端格式。
4. **唯一状态源。** 守护进程的 `StateCache` + `EventBus` 是唯一的
   业务状态；前端不得从 AT 文本推导状态。
5. **UI 冻结。** 布局、样式、文案、页面路由与交互流程在构造上保持不变：
   重构只改变*数据从哪来*，绝不改变画什么。
