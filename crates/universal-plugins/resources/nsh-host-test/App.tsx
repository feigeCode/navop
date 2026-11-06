// nsh 宿主自检页：navop 窗口里挂的扩展页面（M3 起走扩展目录装载路径）。
//
// 这个页面同时是 bundle 的输入：改了它要重新生成 view.js：
//
// ```bash
// cd crates/universal-plugins/resources/nsh-host-test
// nsh build --out . && mv bundle.js view.js
// ```
//
// 三件事一起验：
//
// 1. **页面画得出来**（nsh 自绘 + 宿主写入 nsh 的 shadcn token）——主题跟随
//    宿主：`AppView::set_theme` 送 token，Tailwind 类（bg-card / border-border）
//    取色；明暗跟随 `set_appearance`（`dark:` 变体）。
// 2. **启动事实同步可读**（ABI v2）：`host.params` 在首帧就有 extension /
//    apiVersion / backends / connection，页面不 await 任何 RPC。
// 3. **宿主 ABI 答得上**：用不存在的 backend 开一次资源，宿主必须回带
//    `code` 的结构化错误（不再有 base64 信封），并把它一起回报给宿主的
//    `spike.mounted`，测试台子据此断言「页面真的调到了宿主」。
//
// `nsh.json` 的 `capabilities.host` 在 nsh 引擎侧只是白名单的默认值；宿主
// 装载页面时会用 extension.json 派生的授权覆盖它（ABI v2 §6）。

import { host, locale, mount, Scroll, signal, Text, View } from "nsh";
import { Button, Input } from "nsh/base";

/** 一条会话内自增的记录（页面自己的状态，无宿主交互）。 */
interface Item {
  id: number;
  title: string;
}

/** 宿主写进 `host.params` 的启动事实（ABI v2 §2）。 */
interface Params {
  apiVersion?: number;
  extension?: { id?: string; viewId?: string; viewKey?: string; title?: string };
  backends?: string[];
  connection?: unknown | null;
}

interface Snapshot {
  apiVersion?: number;
  extensionId?: string;
  viewId?: string;
  backends?: string;
  connection?: string;
  /** 能力平面自检：拿一个不存在的 backend 开资源，宿主必须回稳定 code。 */
  resourceErrorCode?: string;
}

const items = signal<Item[]>([]);
const draft = signal("");
const snapshot = signal<Snapshot>({});
let next = 1;

function add() {
  const title = draft().trim();
  if (!title) return;
  items.update((all) => [{ id: next++, title }, ...all]);
  draft.set("");
}

/** 走一遍宿主 ABI，把结果留在页面上（也回报给宿主，供台子断言）。 */
async function probe() {
  const params = (host.params ?? {}) as Params;
  // 能力平面（M4 第二批）：故意用一个不存在的 backend，验证调用真的
  // 走到了 provider 分发层、并且结构化错误带 code 回来。
  let resourceErrorCode = "";
  try {
    await host.call(
      "navop.resource.open",
      { backend: "missing", resourceType: "noop" },
      { timeout: 5_000 },
    );
  } catch (error) {
    const code = (error as { code?: unknown })?.code;
    resourceErrorCode = typeof code === "string" && code !== "" ? code : String(error);
  }
  snapshot.set({
    apiVersion: params.apiVersion,
    extensionId: params.extension?.id,
    viewId: params.extension?.viewId,
    backends: (params.backends ?? []).join(","),
    connection: params.connection == null ? "none" : "present",
    resourceErrorCode,
  });
  host.post("navop.log", {
    level: "info",
    message: "nsh 自检页装载完成",
    fields: { source: "self-check" },
  });
  host.call("spike.mounted", {
    title: "NSH Host",
    items: items().length,
    apiVersion: params.apiVersion,
    extensionId: params.extension?.id,
    viewId: params.extension?.viewId,
    backends: params.backends ?? [],
    hasConnection: params.connection != null,
    locale: locale(),
    resourceErrorCode,
  });
}

function App() {
  void probe();

  return (
    <Scroll style={{ flex: 1 }}>
      <View style={{ padding: 16, gap: 12 }}>
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 8,
          }}
        >
          <Text class="text-lg font-semibold text-foreground">NSH Host</Text>
          <Text class="text-sm text-muted-foreground">navop tab · nsh engine</Text>
        </View>

        <View class="gap-3 rounded-xl border border-border bg-card p-4">
          <View style={{ flexDirection: "row", gap: 8, alignItems: "center" }}>
            <View style={{ flex: 1 }}>
              <Input
                value={draft()}
                placeholder="记点什么…"
                onChange={({ value }) => draft.set(value)}
                onSubmit={add}
                class="min-h-9 rounded-lg border border-input bg-background px-3 text-sm text-card-foreground focus:border-ring"
              />
            </View>
            <Button title="添加" onPress={add} />
          </View>
          {items().length === 0 ? (
            <Text class="text-sm text-muted-foreground">
              还没有记录 —— 输入后点「添加」。
            </Text>
          ) : (
            items().map((it) => (
              <View key={it.id} class="rounded-lg border border-border px-3 py-2">
                <Text class="text-sm font-medium text-card-foreground">{it.title}</Text>
              </View>
            ))
          )}
        </View>

        <View class="gap-1 rounded-xl border border-border px-3 py-2">
          <Text class="text-xs text-muted-foreground">host ABI v2</Text>
          <Text class="text-sm text-card-foreground">
            {snapshot().extensionId ?? "—"} · view {snapshot().viewId ?? "—"}　api{" "}
            {snapshot().apiVersion ?? "—"}
          </Text>
          <Text class="text-sm text-card-foreground">
            语言 · {locale()}　backends · {snapshot().backends || "—"}　连接 ·{" "}
            {snapshot().connection ?? "—"}
          </Text>
          <Text class="text-sm text-card-foreground">
            能力平面 · {snapshot().resourceErrorCode || "—"}
          </Text>
        </View>

        <Text class="text-xs text-muted-foreground">
          第一个跑在 navop 窗口里的 nsh 页面：主题、字体、明暗、语言全部来自宿主。
        </Text>
      </View>
    </Scroll>
  );
}

mount(App);
