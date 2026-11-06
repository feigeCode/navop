import { append, bindProp, component, element, insert, prop, text } from "nsh:rt";
import { host, locale, mount, signal } from "nsh";
import { Button } from "nsh/base";

//#region App.tsx
const items = signal([]);
const draft = signal("");
const snapshot = signal({});
let next = 1;
function add() {
	const title = draft().trim();
	if (!title) return;
	items.update((all) => [{
		id: next++,
		title
	}, ...all]);
	draft.set("");
}
/** 走一遍宿主 ABI，把结果留在页面上（也回报给宿主，供台子断言）。 */
async function probe() {
	const params = host.params ?? {};
	let resourceErrorCode = "";
	try {
		await host.call("navop.resource.open", {
			backend: "missing",
			resourceType: "noop"
		}, { timeout: 5e3 });
	} catch (error) {
		const code = error?.code;
		resourceErrorCode = typeof code === "string" && code !== "" ? code : String(error);
	}
	snapshot.set({
		apiVersion: params.apiVersion,
		extensionId: params.extension?.id,
		route: params.route,
		backends: (params.backends ?? []).join(","),
		connection: params.connection == null ? "none" : "present",
		resourceErrorCode
	});
	host.post("navop.log", {
		level: "info",
		message: "nsh 自检页装载完成",
		fields: { source: "self-check" }
	});
	host.call("spike.mounted", {
		title: "NSH Host",
		items: items().length,
		apiVersion: params.apiVersion,
		extensionId: params.extension?.id,
		route: params.route,
		backends: params.backends ?? [],
		hasConnection: params.connection != null,
		locale: locale(),
		resourceErrorCode
	});
}
function App() {
	probe();
	return (() => {
		const __nsh_hv0 = element("scroll");
		prop(__nsh_hv0, "style", { flex: 1 });
		const __nsh_hv1 = element("view");
		prop(__nsh_hv1, "style", {
			padding: 16,
			gap: 12
		});
		const __nsh_hv2 = element("view");
		prop(__nsh_hv2, "style", {
			flexDirection: "row",
			alignItems: "center",
			justifyContent: "space-between",
			gap: 8
		});
		const __nsh_hv3 = element("text");
		prop(__nsh_hv3, "class", "text-lg font-semibold text-foreground");
		const __nsh_hv4 = text("NSH Host");
		append(__nsh_hv3, __nsh_hv4);
		const __nsh_hv5 = element("text");
		prop(__nsh_hv5, "class", "text-sm text-muted-foreground");
		const __nsh_hv6 = text("navop tab · nsh engine");
		append(__nsh_hv5, __nsh_hv6);
		append(__nsh_hv2, __nsh_hv3);
		append(__nsh_hv2, __nsh_hv5);
		const __nsh_hv7 = element("view");
		prop(__nsh_hv7, "class", "gap-3 rounded-xl border border-border bg-card p-4");
		const __nsh_hv8 = element("view");
		prop(__nsh_hv8, "style", {
			flexDirection: "row",
			gap: 8,
			alignItems: "center"
		});
		const __nsh_hv9 = element("view");
		prop(__nsh_hv9, "style", { flex: 1 });
		const __nsh_hv10 = element("input");
		bindProp(__nsh_hv10, "value", () => draft());
		prop(__nsh_hv10, "placeholder", "记点什么…");
		prop(__nsh_hv10, "onChange", ({ value }) => draft.set(value));
		prop(__nsh_hv10, "onSubmit", add);
		prop(__nsh_hv10, "class", "min-h-9 rounded-lg border border-input bg-background px-3 text-sm text-card-foreground focus:border-ring");
		append(__nsh_hv9, __nsh_hv10);
		append(__nsh_hv8, __nsh_hv9);
		insert(__nsh_hv8, component(Button, {
			title: "添加",
			onPress: add
		}));
		append(__nsh_hv7, __nsh_hv8);
		insert(__nsh_hv7, () => items().length === 0 ? (() => {
			const __nsh_hv11 = element("text");
			prop(__nsh_hv11, "class", "text-sm text-muted-foreground");
			const __nsh_hv12 = text("还没有记录 —— 输入后点「添加」。");
			append(__nsh_hv11, __nsh_hv12);
			return __nsh_hv11;
		})() : items().map((it) => (() => {
			const __nsh_hv13 = element("view");
			bindProp(__nsh_hv13, "key", () => it.id);
			prop(__nsh_hv13, "class", "rounded-lg border border-border px-3 py-2");
			const __nsh_hv14 = element("text");
			prop(__nsh_hv14, "class", "text-sm font-medium text-card-foreground");
			insert(__nsh_hv14, () => it.title);
			append(__nsh_hv13, __nsh_hv14);
			return __nsh_hv13;
		})()));
		const __nsh_hv15 = element("view");
		prop(__nsh_hv15, "class", "gap-1 rounded-xl border border-border px-3 py-2");
		const __nsh_hv16 = element("text");
		prop(__nsh_hv16, "class", "text-xs text-muted-foreground");
		const __nsh_hv17 = text("host ABI v2");
		append(__nsh_hv16, __nsh_hv17);
		const __nsh_hv18 = element("text");
		prop(__nsh_hv18, "class", "text-sm text-card-foreground");
		const __nsh_hv19 = text(" · view ");
		const __nsh_hv20 = text("　api");
		append(__nsh_hv18, __nsh_hv19);
		append(__nsh_hv18, __nsh_hv20);
		insert(__nsh_hv18, () => snapshot().extensionId ?? "—", __nsh_hv19);
		insert(__nsh_hv18, () => snapshot().viewId ?? "—", __nsh_hv20);
		insert(__nsh_hv18, [" ", () => snapshot().apiVersion ?? "—"]);
		const __nsh_hv21 = element("text");
		prop(__nsh_hv21, "class", "text-sm text-card-foreground");
		const __nsh_hv22 = text("语言 · ");
		const __nsh_hv23 = text("　backends · ");
		const __nsh_hv24 = text("　连接 ·");
		append(__nsh_hv21, __nsh_hv22);
		append(__nsh_hv21, __nsh_hv23);
		append(__nsh_hv21, __nsh_hv24);
		insert(__nsh_hv21, () => locale(), __nsh_hv23);
		insert(__nsh_hv21, () => snapshot().backends || "—", __nsh_hv24);
		insert(__nsh_hv21, [" ", () => snapshot().connection ?? "—"]);
		const __nsh_hv25 = element("text");
		prop(__nsh_hv25, "class", "text-sm text-card-foreground");
		const __nsh_hv26 = text("能力平面 · ");
		append(__nsh_hv25, __nsh_hv26);
		insert(__nsh_hv25, () => snapshot().resourceErrorCode || "—");
		append(__nsh_hv15, __nsh_hv16);
		append(__nsh_hv15, __nsh_hv18);
		append(__nsh_hv15, __nsh_hv21);
		append(__nsh_hv15, __nsh_hv25);
		const __nsh_hv27 = element("text");
		prop(__nsh_hv27, "class", "text-xs text-muted-foreground");
		const __nsh_hv28 = text("第一个跑在 navop 窗口里的 nsh 页面：主题、字体、明暗、语言全部来自宿主。");
		append(__nsh_hv27, __nsh_hv28);
		append(__nsh_hv1, __nsh_hv2);
		append(__nsh_hv1, __nsh_hv7);
		append(__nsh_hv1, __nsh_hv15);
		append(__nsh_hv1, __nsh_hv27);
		append(__nsh_hv0, __nsh_hv1);
		return __nsh_hv0;
	})();
}
mount(App);

//#endregion