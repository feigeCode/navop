# llm-connector Roadmap

基于对 `rust-genai` 的深入分析，我们将按照以下四个阶段对 `llm-connector` 进行功能增强和架构升级，旨在打造一个更强大、更灵活、更具生产力的 Rust LLM 基础设施。

## 📅 Phase 1: 推理能力增强 (Reasoning & Thinking)
**目标**: 全面支持 DeepSeek R1, Claude 3.7 Sonnet, Gemini 1.5 Pro 等模型的推理/思考能力控制。

- [x] **Feature**: 在 `ChatRequest` 中添加 `reasoning_effort` 字段 (Low, Medium, High)。
  - 对应 Claude 3.7 的 `reasoning_effort` 参数。
  - 对应 Gemini 的 Thinking Levels。
- [x] **Feature**: 在 `ChatRequest` 中添加 `budget_tokens` 字段 (用于控制思考 token 上限)。
  - 对应 Claude 3.7 的 `thinking: { budget_tokens: N }`。
- [x] **Adapter**: 更新 `AnthropicProtocol` 适配 Claude 3.7 的 `thinking` 参数。
  - 实现 `thinking` 参数映射逻辑。
- [x] **Adapter**: 更新 `GoogleProtocol` 适配 Gemini 的 `thinking` 参数。
- [x] **Test**: 添加推理参数的集成测试。

## 📅 Phase 2: 动态配置与路由 (Dynamic Configuration)
**目标**: 支持运行时动态决定 Endpoint、Key 和 Model，适应网关和多租户场景。

- [x] **Refactor**: 引入 `ServiceResolver` trait (类似 rust-genai 的 `ServiceTargetResolver`)。
  - 允许用户注入自定义逻辑，在每次请求前动态修改配置。
- [x] **Feature**: 实现基于 Model Name 的动态路由 (基础框架)。
  - 例如: `openai/gpt-4` -> Azure, `deepseek/r1` -> Local Ollama。
- [x] **Feature**: 支持从环境变量或外部配置源动态加载 Key。
  - 实现 `EnvKeyResolver`。

## 📅 Phase 3: 流式引擎增强 (Robust Streaming)
**目标**: 提升对非标准 SSE 流的兼容性，处理网络抖动。

- [x] **Refactor**: 调研并重构 SSE 解析模块。
  - 实现 `create_text_stream` 支持 Auto/SSE/NDJSON 多种格式。
  - 增强对不规范 SSE 格式 (如 Ollama, Zhipu) 的容错能力。
- [x] **Fix**: 处理厂商流式输出中的特殊情况。
  - Ollama 使用 NDJSON。
  - Zhipu 使用单换行 SSE。
- [ ] **Test**: 增加流式断连重试机制的测试。

## 📅 Phase 4: 开发者体验 (DX Improvements)
**目标**: 简化文件上传和多模态交互。

- [x] **Utils**: 添加 `MessageBlock::from_file_path()` 辅助函数。
  - 支持自动检测 MIME 类型 (Image/PDF)。
  - 自动 Base64 编码。
- [ ] **Utils**: 自动处理 PDF/Image 的 Base64 编码和 MIME type 识别。
- [ ] **Docs**: 更新文档，增加高级用法示例。
  - 包括推理参数使用、动态路由配置、本地文件上传等场景。
