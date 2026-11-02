// nsh 宿主自检页：navop 窗口里挂的扩展页面（M3 起走扩展目录装载路径）。
//
// 这个页面同时是 bundle 的输入：改了它要重新生成 view.js：
//
// ```bash
// cd crates/universal-plugins/resources/nsh-host-test
// nsh build --out . && mv bundle.js view.js
// ```
//
//
// 这是 embed-gpui 官方示例（Order Ticket）的精简版：去掉了宿主
// nativeModule/nativeComponent（`host/trading`、`host.switch`、
// `host.watchlist`），只保留 `nsh` / `nsh/base` 自足的部分 —— spike 阶段
// 宿主还不注册任何 native 模块，页面必须完全自持。
//
// 主题跟随宿主：AppView::set_theme 推来的 shadcn token 供 Tailwind 类
// （bg-card / border-border / text-primary-foreground）取色；明暗跟随
// set_appearance（`dark:` 变体）。

import { host, mount, Scroll, signal, Text, View } from "nsh";
import { Button, Input } from "nsh/base";

/** 一条会话内自增的记录（spike 自己的状态，无宿主交互）。 */
interface Item {
  id: number;
  title: string;
}

const items = signal<Item[]>([]);
const draft = signal("");
let next = 1;

function add() {
  const title = draft().trim();
  if (!title) return;
  items.update((all) => [{ id: next++, title }, ...all]);
  draft.set("");
}

function App() {
  // M2 的宿主动手：页面真挂到这个点才算跑起来了（测试与冒烟都看这条）。
  // 能力在 capabilities.json 里声明为 host: ["spike.mounted"]，未声明会被
  // nsh 的 host 白名单直接拒掉。
  host.call("spike.mounted", { title: "NSH Host", items: items().length });

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
          <Text class="text-lg font-semibold text-foreground">NSH Spike</Text>
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

        <Text class="text-xs text-muted-foreground">
          第一个跑在 navop 窗口里的 nsh 页面：主题、字体、明暗全部来自宿主。
        </Text>
      </View>
    </Scroll>
  );
}

mount(App);
