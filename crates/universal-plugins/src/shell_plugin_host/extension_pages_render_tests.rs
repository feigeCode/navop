//! 常驻守卫：扩展的 shell 页面（`navop-extensions/extensions/composite/*/ui/*.js`）
//! 必须能在 `gpui-component` 的组件目录下渲染，且输入控件来自 `gpui-component`
//! 而**状态仍来自 `gpui-base`**（元素换、状态不换）。
//!
//! 为什么不是静态检查：组件调用契约（构造签名、方法名、state 形状、主题 token 名）
//! 的错误只有在真渲染时才暴露，`node --test` 与 diff 都看不见。
//! 判据与台子的由来见技能 `navop-middleware-ui-gap-diagnose` §1.11 / §1.12。
//!
//! 与 `shell_plugin_host` 的其它测试不同，这里**不借用真实工作台会话**：
//! 页面在这个台子里只需要能画出来，`navop.workbench` / `navop.dev` / `navop.log`
//! 一律用返回空数据的桩替代，所以不需要连接、不需要 catalog。
//!
//! 需要 `--features shell-plugins`（`gpui-shell` / `gpui-component-shell` 是可选依赖）：
//! ```bash
//! cargo test -p universal-plugins --features shell-plugins -- extension_pages_render
//! ```

#[cfg(test)]
mod tests {
    use std::{
        fs,
        ops::Deref as _,
        path::{Path, PathBuf},
        rc::Rc,
    };

    use gpui::{Entity, IntoElement as _, TestAppContext, VisualTestContext};
    use gpui_shell::policy::Policy;
    use gpui_shell::{HostArguments, HostModule, HostResult, HostValue, ViewLoadOptions};

    /// 已迁移的页面：入口相对路径 + 必须物化出来的组件节点名。
    ///
    /// `Input` / `Textarea` 是这次迁移的对象（元素从 `gpui-component` 取、
    /// 状态留在 `gpui-base`），其余是同一批文件里本来就来自组件库的控件。
    const MIGRATED_PAGES: &[(&str, &[&str])] = &[
        ("mqtt/ui/messages.js", &["Input", "Select", "Button"]),
        ("mqtt/ui/publish.js", &["Input", "Textarea", "Switch"]),
        ("mqtt/ui/subscriptions.js", &["Input", "Select"]),
        ("rocketmq/ui/send-message.js", &["Input"]),
        ("docker/ui/log-viewer.js", &["Input"]),
        ("dev-tools/ui/workbench.js", &["Input"]),
    ];

    /// 扩展源在相邻仓 `navop-extensions`。navop 的构建/CI 不保证 check out 它，
    /// 所以缺席时**跳过**而不是失败 —— 扩展自己的静态守卫
    /// （`navop-extensions/tests/scripts.test.mjs`）与文件同处一地，那边永远会跑。
    fn extension_ui_root() -> Option<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../navop-extensions/extensions/composite");
        root.is_dir().then_some(root)
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap_or_else(|e| panic!("create {}: {e}", to.display()));
        for entry in fs::read_dir(from).unwrap_or_else(|e| panic!("read {}: {e}", from.display())) {
            let entry = entry.expect("directory entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("file type").is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).expect("copy source file");
            }
        }
    }

    /// 把 `ui` 目录整份复制过去，让 `./shared.js` 这类相对导入按模块所在目录解析。
    fn copy_extension_sources(root: &Path) -> PathBuf {
        let source = extension_ui_root().expect("extension sources");
        for relative in ["mqtt/ui", "rocketmq/ui", "docker/ui", "dev-tools/ui"] {
            copy_dir(&source.join(relative), &root.join(relative));
        }
        root.to_path_buf()
    }

    /// 替代 `navop.workbench` / `navop.dev` / `navop.log`。
    ///
    /// 返回值按各页面已经能处理的形状给：`dispatch` 为 `Null`（页面都写成
    /// `result?.x || []`），但 `navop.dev` 的 `list` / `logs` **必须是空数组** ——
    /// dev-tools 的 `render` 直接对结果调 `.filter()`，`Null` 会让它整页 build_error。
    fn extension_policy() -> Rc<Policy> {
        let null: fn(&HostArguments) -> HostResult = |_| Ok(HostValue::Null);
        let empty: fn(&HostArguments) -> HostResult = |_| Ok(HostValue::Array(Vec::new()));

        let workbench = HostModule::new("navop.workbench")
            .function("current", null)
            .function("dispatch", null);
        let dev = HostModule::new("navop.dev")
            .function("list", empty)
            .function("logs", empty)
            .function("open", null)
            .function("openView", null)
            .function("pickDirectory", null)
            .function("pickResult", null)
            .function("reload", null)
            .function("remove", null)
            .function("watch", null);
        let log = HostModule::new("navop.log")
            .function("info", null)
            .function("error", null);

        Rc::new(
            Policy::new()
                .with_host_module(workbench)
                .expect("`navop.workbench` is not a reserved specifier")
                .with_host_module(dev)
                .expect("`navop.dev` is not a reserved specifier")
                .with_host_module(log)
                .expect("`navop.log` is not a reserved specifier"),
        )
    }

    struct Empty;

    impl gpui::Render for Empty {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    /// 加载一个入口。返回的 `LoadedScriptView` 必须活到断言结束 —— drop 即退休。
    fn load(
        root: &Path,
        entry: &str,
        cx: &mut TestAppContext,
    ) -> (
        VisualTestContext,
        gpui_shell::LoadedScriptView,
        Rc<gpui_shell::ShellRuntime>,
    ) {
        cx.update(|cx| {
            gpui_component_shell::init(cx);
        });
        let runtime = gpui_component_shell::new_isolated_runtime().expect("isolated runtime");
        let options = ViewLoadOptions::new(root.to_path_buf(), entry, extension_policy());
        let window = cx.add_window(|_, _| Empty);
        let mut context = VisualTestContext::from_window(*window.deref(), cx);
        let loaded = context
            .update(|window, cx| runtime.load_view(options, window, cx))
            .unwrap_or_else(|error| panic!("{entry} 必须能加载: {error:#}"));
        (context, loaded, runtime)
    }

    fn draw_once(context: &mut VisualTestContext, view: &Entity<gpui_shell::ScriptView>) {
        context.draw(
            gpui::Point::default(),
            gpui::size(gpui::px(1024.), gpui::px(768.)),
            {
                let view = view.clone();
                move |_, _| view.into_any_element()
            },
        );
    }

    /// 画一帧 → 泵执行器 → 再画一帧。
    ///
    /// 第二帧是必需的：`init` 里 `cx.spawn` 出去的加载（`this.load(cx)`、轮询）
    /// **不会被 `draw()` 推进**，只泵一次才从"加载中"分支走到数据分支
    /// （`subscriptions.js` 不泵就永远停在 Spinner）。`VisualTestContext` derefs 到
    /// `TestAppContext`，所以 `run_until_parked()` 可以直接调。
    fn render_settled(
        context: &mut VisualTestContext,
        loaded: &gpui_shell::LoadedScriptView,
        entry: &str,
    ) -> (Option<String>, String) {
        let view: Entity<gpui_shell::ScriptView> = loaded.view().clone();
        draw_once(context, &view);
        context.run_until_parked();
        draw_once(context, &view);
        context.run_until_parked();
        context.update(|_, cx| {
            let view = view.read(cx);
            match (view.build_error(), view.snapshot()) {
                (Some(error), _) => (Some(error.to_owned()), String::new()),
                (None, Some(snapshot)) => (None, snapshot.debug_tree()),
                (None, None) => (Some(format!("{entry} 没有产出快照")), String::new()),
            }
        })
    }

    /// `tree` 里节点名恰好是 `name` 的那些行（已去缩进）。`name` 必须在词边界断开，
    /// 否则 `Input` 会匹配到 `InputGroup`。
    fn node_lines<'a>(tree: &'a str, name: &str) -> impl Iterator<Item = &'a str> {
        tree.lines().map(str::trim_start).filter(move |line| {
            line.strip_prefix(name)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
    }

    /// 树里的 `name` 是 **gpui-base 元素**（即没迁移）时返回那一行。
    ///
    /// `shell/src/spec.rs` 给 `Component::Input(_) / Textarea / NumberInput / OtpInput`
    /// 打印 ` #{handle}`，而组件库注册表元素是 `Component::Registered(..)`，打印裸名字
    /// （调过注册方法再加 `:name(registered)`）。所以 handle 就是"还是旧元素"的判据：
    /// `Input.new(state)` → `Input #4294967296`，`new Input(state)` → `Input`。
    ///
    /// 只断 `tree.contains("Input")` 两种写法都通过，等于没断。
    fn gpui_base_element(tree: &str, name: &str) -> Option<String> {
        node_lines(tree, name)
            .find(|line| line[name.len()..].trim_start().starts_with('#'))
            .map(str::to_owned)
    }

    /// 树里的 `name` 是不是**组件库注册表元素**（即已迁移）。
    fn registered_element(tree: &str, name: &str) -> Option<String> {
        node_lines(tree, name)
            .find(|line| !line[name.len()..].trim_start().starts_with('#'))
            .map(str::to_owned)
    }

    /// 迁移契约本身，不依赖相邻仓：`gpui-base` 的 state 喂给 `gpui-component` 的
    /// 元素必须能渲染，且元素在树里必须**不带 handle**（= 来自注册表）。
    ///
    /// `aria_label` 是组件 `Input` 注册表里仅有的两个方法之一
    /// （`crates/component-shell/src/shell/retained_forms/mod.rs`），`Textarea` 的在
    /// `crates/component-shell/src/shell/layout/textarea.rs`；渲染树把注册方法标成
    /// `(registered)`，所以那条标记本身就是"元素来自组件库"的证据。
    #[gpui::test]
    fn component_element_accepts_a_gpui_base_state(cx: &mut TestAppContext) {
        let migrated = r#"
import { View, div } from "gpui-kit";
import { v_flex, InputState, TextareaState } from "gpui-base";
import { Input, Textarea } from "gpui-component";

export default class Migrated extends View {
  init() {
    this.topic = InputState.new({ value: "topic", placeholder: "topic" });
    this.body = TextareaState.new({ value: "body", rows: 4 });
  }
  render() {
    return v_flex()
      .child(new Input(this.topic).aria_label("probe-topic"))
      .child(new Textarea(this.body).aria_label("probe-body"))
      .child(div().child(`read=${this.topic.value()}|${this.body.value()}`));
  }
}
"#;
        let root = tempfile::tempdir().expect("temp root");
        fs::write(root.path().join("main.js"), migrated).expect("write entry");

        let (mut context, loaded, _runtime) = load(root.path(), "main.js", cx);
        let (error, tree) = render_settled(&mut context, &loaded, "main.js");
        assert_eq!(error, None, "元素用组件库、状态用 gpui-base 必须能渲染:\n{tree}");
        assert!(
            tree.contains("Input :aria_label(registered)"),
            "Input 必须来自组件库注册表:\n{tree}"
        );
        assert!(
            tree.contains("Textarea :aria_label(registered)"),
            "Textarea 必须来自组件库注册表:\n{tree}"
        );
        assert!(
            tree.contains("read=topic|body"),
            "gpui-base 的 state 必须仍能回答 value():\n{tree}"
        );
    }

    /// 上一条的对照：同一形状全用 `gpui-base` 构造时，树里是带 handle 的元素。
    ///
    /// 没有这条，"页面里没有 `Input #`"这个断言就不知道在排除什么。
    #[gpui::test]
    fn gpui_base_elements_are_printed_with_a_handle(cx: &mut TestAppContext) {
        let base = r#"
import { View } from "gpui-kit";
import { v_flex, Input, InputState, Textarea, TextareaState } from "gpui-base";

export default class Base extends View {
  init() {
    this.topic = InputState.new({ value: "topic" });
    this.body = TextareaState.new({ value: "body", rows: 4 });
  }
  render() {
    return v_flex()
      .child(Input.new(this.topic))
      .child(Textarea.new(this.body));
  }
}
"#;
        let root = tempfile::tempdir().expect("temp root");
        fs::write(root.path().join("main.js"), base).expect("write entry");

        let (mut context, loaded, _runtime) = load(root.path(), "main.js", cx);
        let (error, tree) = render_settled(&mut context, &loaded, "main.js");
        assert_eq!(error, None, "gpui-base 的写法本身仍然可用:\n{tree}");
        assert!(
            gpui_base_element(&tree, "Input").is_some(),
            "`Input.new(state)` 应该打印带 handle 的元素:\n{tree}"
        );
        assert!(
            gpui_base_element(&tree, "Textarea").is_some(),
            "`Textarea.new(state)` 应该打印带 handle 的元素:\n{tree}"
        );
    }

    /// 真的扩展页面：加载、渲染、零 build_error，且 `Input` / `Textarea`
    /// **不带 handle**（即不是回退后的 gpui-base 元素）。
    #[gpui::test]
    fn extension_pages_render_with_component_elements(cx: &mut TestAppContext) {
        let Some(_) = extension_ui_root() else {
            eprintln!(
                "SKIP extension_pages_render_with_component_elements: \
                 ../../../navop-extensions/extensions/composite 不存在\
                 （扩展的静态守卫在 navop-extensions 仓内，那边总会跑）"
            );
            return;
        };
        let root = tempfile::tempdir().expect("temp root");
        let root = copy_extension_sources(root.path());
        let mut failures: Vec<String> = Vec::new();

        for (entry, expected) in MIGRATED_PAGES {
            let (mut context, loaded, _runtime) = load(&root, entry, cx);
            let (error, tree) = render_settled(&mut context, &loaded, entry);
            if let Some(error) = error {
                failures.push(format!("{entry}: 渲染失败: {error}"));
                continue;
            }
            for needle in *expected {
                if let Some(base) = gpui_base_element(&tree, needle) {
                    failures.push(format!(
                        "{entry}: `{base}` 是 gpui-base 元素，不是 gpui-component:\n{tree}"
                    ));
                } else if registered_element(&tree, needle).is_none() {
                    failures.push(format!("{entry}: 树里没有 `{needle}` 节点:\n{tree}"));
                }
            }
            if !tree.contains("(registered)") {
                failures.push(format!(
                    "{entry}: 树里没有任何注册方法，组件目录可能没供上这一页:\n{tree}"
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "{} / {} 个扩展页面不符合迁移后的形态:\n{}",
            failures.len(),
            MIGRATED_PAGES.len(),
            failures.join("\n\n")
        );
    }
}
