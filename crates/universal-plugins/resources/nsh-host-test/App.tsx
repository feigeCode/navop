// nsh 宿主自检页：navop 窗口里挂的扩展页面（M3 起走扩展目录装载路径）。
//
// 这个页面同时是 bundle 的输入：改了它要重新生成 view.js：
//
// ```bash
// cd crates/universal-plugins/resources/nsh-host-test
// nsh build --out . && mv bundle.js view.js
// ```
//
// 两件事一起验：
//
// 1. **页面画得出来**（nsh 自绘 + 宿主推来的 shadcn token）——主题跟随
//    宿主：`AppView::set_theme` 送 token，Tailwind 类（bg-card / border-border）
//    取色；明暗跟随 `set_appearance`（`dark:` 变体）。
// 2. **宿主 ABI 答得上**（M4）：启动时把 `navop.context.current`、
//    `navop.i18n.locale`、`navop.theme.snapshot` 的应答一起回报给宿主，
//    测试台子据此断言"页面真的调到了宿主，且拿到的是宿主的值"。
//
// `nsh.json` 的 `capabilities.host` 必须列出这些名字（含通配），否则 nsh 的
// host 白名单在引擎侧就拒掉，根本到不了宿主。

import { host, mount, Scroll, signal, Text, View } from "nsh";
import { Button, Input } from "nsh/base";

/** 一条会话内自增的记录（页面自己的状态，无宿主交互）。 */
interface Item {
  id: number;
  title: string;
}

interface Snapshot {
  viewId?: string;
  locale?: string;
  themeRevision?: number;
  dark?: boolean;
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
async function probe(host_) {
  const context = await host_.call("navop.context.current", {});
  const theme = await host_.call("navop.theme.snapshot", {});
  const locale = await host_.call("navop.i18n.locale", {});
  snapshot.set({
    viewId: context?.viewId,
    locale,
    themeRevision: theme?.revision,
    dark: theme?.dark,
  });
  host_.call("navop.log.info", { message: "nsh 自检页装载完成" });
  host_.call("spike.mounted", {
    title: "NSH Host",
    items: items().length,
    viewId: context?.viewId,
    locale,
    themeRevision: theme?.revision,
  });
}

function App() {
  void probe(host);

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
          <Text class="text-xs text-muted-foreground">host ABI</Text>
          <Text class="text-sm text-card-foreground">
            view · {snapshot().viewId ?? "—"}　语言 · {snapshot().locale ?? "—"}　
            主题 · rev {snapshot().themeRevision ?? "—"}
            {snapshot().dark ? "（暗）" : "（亮）"}
          </Text>
        </View>

        <Text class="text-xs text-muted-foreground">
          第一个跑在 navop 窗口里的 nsh 页面：主题、字体、明暗全部来自宿主。
        </Text>
      </View>
    </Scroll>
  );
}

mount(App);
