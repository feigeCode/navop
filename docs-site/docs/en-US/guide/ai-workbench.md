# AI workbench, Agents, and ACP

The AI workbench places model conversations beside live databases, terminals, files, and Notes. Model output can be wrong, while tool calls can perform real operations. Select Ask, Plan, or Auto according to risk and keep human verification in the loop.

## Configure an LLM provider

Add a Provider in Settings with its name, API key, base URL, API version, and other required values. Load available models or define a custom model, enable or disable providers, edit them, and choose a default. Validate compatibility and data policy for enterprise proxies or OpenAI-compatible endpoints.

Never paste API keys into conversations or reports. Confirm every base URL before sending prompts or attachments. Tune history length, temperature, and max tokens for context, cost, and repeatability.

## Choose Ask, Plan, or Auto

Ask is appropriate for explanation and drafts. Plan organizes steps for review. Auto can continue through more actions within granted permissions. Prefer Ask or Plan for production databases, terminal writes, and file replacement.

Review generated SQL and commands before execution and compare explanations with the real schema, sample data, and server output.

## Attach resources and Skills

Use `@` to reference available connections, query results, files, or resource-pool entries, and attach images when useful. Provide only the minimum data needed; names, result rows, and screenshots may be sensitive.

Skills supply focused workflows and rules. Review their source and permissions before import, manage or remove them as needed, and re-evaluate behavior after updates. A Skill may request tools but cannot bypass Navop permissions.

## Manage sessions and approvals

Create, rename, archive, and delete conversations or tasks. Archiving is organization, not a guarantee that an external model provider has erased previously transmitted data. Tool views show calls, plans, and approvals; inspect tool name, resource, parameters, and side effects.

Agents may use plans or sub-agents, but their results still require verification. Recheck connection and file state before acting on a long-running plan.

## Use ACP and HTML preview

ACP extensions connect external Agents and may request login and permissions. Externally managed tasks are not necessarily persisted by Navop. Even after ACP authorization, a Public MCP call can require a second Navop approval; this is an intentional host-security boundary.

HTML code blocks can be copied, downloaded, opened in a browser, or previewed in-app. Source remains available if WebView cannot run. Inspect unknown HTML first because preview may load external resources or execute scripts; it is not a guaranteed sandbox.
