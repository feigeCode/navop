# AI automation

navop 的 issue 机器人：自动分诊 + 可选自动改码 + 评审闭环。

- **引擎**：`scripts/ai/agent.cjs`，默认使用 Codex
- **规则**：`scripts/ai/automation.cjs`（纯函数，19 个单测覆盖）
- **流程**：`.github/workflows/ai-automation.yml`
- **提示词**：本目录下 `prompts/`，**改提示词不需要改 workflow**

---

## 1. 配置密钥

Settings → Secrets and variables → Actions。

### Secrets

| Secret | 必填 | 用途 |
|---|---|---|
| `AI_API_KEY` | ✅ | 模型后端密钥 |
| `TRIAGE_GITHUB_TOKEN` | 可选 | 机器人身份 PAT。不填则用 `github-actions[bot]` |
| `BRAVE_API_KEY` | 可选 | 联网研究。不填则跳过外部研究阶段 |

用 `gh secret set AI_API_KEY` 设置，不要写进仓库。

### Variables

| Variable | 默认 | 说明 |
|---|---|---|
| `AI_PROVIDER` | `codex` | `codex` 或 `openai-compatible` |
| `AI_BASE_URL` | `https://api.openai.com/v1` | 按 provider 自动取默认值 |
| `AI_MODEL` | Codex CLI 默认模型 | 需要指定时填写模型名 |
| `AI_AUTOMATION_MODE` | `full` | 改成 `triage_only` 只分诊、不改码 |
| `AI_VERIFY_COMMAND` | `cargo check --workspace --all-targets` | 改码后的验证命令 |
| `AI_MAX_REVIEW_ROUNDS` | `5` | 评审闭环最大轮数，超过就交人 |
| `AI_TRIAGE_DAILY_LIMIT` | `10` | 每日自动分诊预算 |
| `AI_OWN_ACTORS` | `github-actions[bot]` | 视为「自己人」的账号，逗号分隔 |

---

## 2. 选后端

### A. `codex`（默认，能力完整）

用 Codex CLI 连接 OpenAI API。有完整仓库工具链，分类使用只读 sandbox，自动改码使用
workspace-write sandbox。**只有这个后端支持自动改码**。

| 场景 | `AI_BASE_URL` | `AI_MODEL` | `AI_API_KEY` |
|---|---|---|---|
| OpenAI 官方 | `https://api.openai.com/v1` | Codex CLI 默认模型 | OpenAI API key |
| OpenAI 兼容网关 | 网关地址 | 网关支持的 Codex 模型名 | 网关 key |

### B. `openai-compatible`（便宜，能力受限）

直连 `/v1/chat/completions` + `response_format=json_object`。脚本会先用 `rg`
抓相关文件塞进上下文。**能分诊，不能自动改码**（改码步骤会自动跳过并交人）。

| 场景 | `AI_BASE_URL` | `AI_MODEL` |
|---|---|---|
| OpenAI 官方 | `https://api.openai.com/v1` | `gpt-4o-mini` |
| DeepSeek | `https://api.deepseek.com/v1` | `deepseek-chat` |
| 其他兼容网关 | 网关地址 | 网关模型名 |

---

## 3. 初始化标签

```bash
gh label create "triage"                        -c "#ededed" -d "已由自动化分诊" || true
gh label create "triage:bug-ready"              -c "#d73a4a" || true
gh label create "triage:bug-needs-info"         -c "#fbca04" || true
gh label create "triage:feature-quick-win"      -c "#a2eeef" || true
gh label create "triage:feature-defer"          -c "#c5def5" || true
gh label create "triage:already-available"      -c "#0e8a16" || true
gh label create "triage:unclear"                -c "#cccccc" || true
gh label create "triage:other"                  -c "#cccccc" || true
gh label create "ready-for-agent"               -c "#5319e7" || true
gh label create "ready-for-human"               -c "#b60205" || true
gh label create "needs-info"                    -c "#fbca04" || true
gh label create "invalid-format"                -c "#e4e669" || true
gh label create "automation:bot-pr"             -c "#5319e7" || true
gh label create "automation:review-loop"        -c "#d4c5f9" || true
gh label create "automation:review-clean"       -c "#0e8a16" || true
```

或直接跑 `bash scripts/ai/create-labels.sh`。

---

## 4. 流程

```
issues / issue_comment / pull_request_target / cron / 手动
        │
        ▼
   route（纯 JS，不调模型）
        ├─ issue_classify  → classify → 打标签 + 回复 → 可自动修则 implement
        ├─ issue_followup  → 已在 follow-up 分支处理（当前版本重新分类）
        ├─ review_loop     → 读评审意见 → 修 / 标记 clean / 超轮数交人
        ├─ cleanup         → PR 关闭后清理源 issue
        └─ skip
```

`implement` 跑在**不可信 runner**，只产出 patch 产物；`publish_pr` 在**全新 runner**
应用 patch 并开 **draft PR**（永不自动合并）。

分类为 `already_available` / `unclear` 时会自动关闭 issue（附 how-to 或说明）。

---

## 5. 安全边界

- 控制面脚本 checkout 后立刻复制到 `$RUNNER_TEMP` 并 `chmod a-w` 冻结，agent 改不了判定规则
- agent 步骤里 `GITHUB_TOKEN=''`、`GH_TOKEN=''`，密钥只经文件注入启动器
- 每步输出都跑凭据泄漏扫描，命中直接失败
- 受保护路径：`.github/**`、`scripts/ai/**`、`Cargo.lock`、`rust-toolchain.toml`、`.env*`、
  `nix/**`、`packaging/**`、`script/release*` —— agent 一律不许改
- fork PR 只走 `pull_request_target` 的元数据处理，不 checkout、不执行、不提交
- 任何环节失败 → 打 `ready-for-human` 标签，不等人发现
- 每日分诊预算 + HTML 注释 watermark 去重，防重复处理

---

## 6. 本地验证

```bash
node --test scripts/ai/automation.test.cjs
node scripts/ai/spam-filter.cjs "download the attached file.zip" NONE
```

手动触发一次分诊：Actions → **AI automation** → Run workflow → 填 issue 号。

## 7. 调优入口

| 想改什么 | 改哪里 |
|---|---|
| 回复语气 / 分类标准 | `prompts/classify.md` |
| 改码约束 | `prompts/implement.md` |
| 分类字段 | `schemas/classification.schema.json` |
| 标签映射 / 受保护路径 / 路由规则 | `scripts/ai/automation.cjs` |
| 模型后端行为 | `scripts/ai/agent.cjs` |
| 验证命令 / 并发 / 限流 | workflow 顶部 `env` + Variables |
