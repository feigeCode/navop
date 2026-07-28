use declarative_ui_demo::{
    CompileOptions, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    DeclarativeView, DeclarativeViewConfig, RenderContext, Runtime, StateStore, compile_template,
};
use gpui::{
    AppContext, Bounds, IntoElement, ParentElement, QuitMode, Styled, TitlebarOptions,
    WindowBounds, WindowOptions, div, px, rgb, size,
};
use gpui_component::Root;
use gpui_component_assets::Assets;

const WINDOW_WIDTH_PX: f32 = 1180.0;
const WINDOW_HEIGHT_PX: f32 = 820.0;
const WINDOW_MIN_WIDTH_PX: f32 = 760.0;
const WINDOW_MIN_HEIGHT_PX: f32 = 560.0;
const SQL_EDITOR_GAP_PX: f32 = 8.0;
const MUTED_TEXT_RGB: u32 = 0xa1_a1_aa;
const PRIMARY_TEXT_RGB: u32 = 0xf4_f4_f5;

const DEMO_HTML: &str = r#"
<div class="flex flex-col h-full min-h-0 overflow-hidden bg-zinc-950 text-zinc-100">
    <header class="flex flex-col gap-2 p-4 flex-shrink-0 border border-zinc-800">
        <span class="text-xl font-semibold">Declarative UI Component Showcase</span>
        <span class="text-sm text-zinc-400">
            Restricted HTML → VNode → Runtime → native gpui-component
        </span>
        <span class="text-sm text-zinc-400">
            No JavaScript · typed diagnostics · transactional actions · string state
        </span>
    </header>

    <main class="flex-1 min-h-0 overflow-y-scroll">
        <div class="flex flex-col gap-4 p-4">
            <alert
                id="runtime-status"
                bind="status_message"
                title="Runtime status"
                variant="info"
                size="sm"
            ></alert>

            <group-box title="Profile and automation" variant="outline">
                <form layout="vertical" columns="2" size="sm" class="w-full">
                    <field label="Username" required>
                        <input
                            id="username-input"
                            bind="username"
                            placeholder="Database administrator"
                            cleanable
                            class="w-full"
                        />
                    </field>
                    <field label="Email" required>
                        <input
                            id="email-input"
                            bind="email"
                            placeholder="admin@example.com"
                            class="w-full"
                        />
                    </field>
                    <field
                        label="Notes"
                        description="Textarea uses the same bidirectional string binding contract."
                        col-span="2"
                    >
                        <textarea
                            id="notes-input"
                            bind="notes"
                            placeholder="Operational notes"
                            cleanable
                            class="w-full"
                        ></textarea>
                    </field>
                    <field label="Notifications" label-indent="false">
                        <checkbox id="notifications" bind="notifications">
                            Email alerts
                        </checkbox>
                    </field>
                    <field label="Synchronization" label-indent="false">
                        <switch id="auto-sync" bind="auto_sync">
                            Auto-sync metadata
                        </switch>
                    </field>
                    <field label="Preview channel" label-indent="false">
                        <radio id="beta-mode" bind="beta_mode">
                            Enable beta mode
                        </radio>
                    </field>
                    <field label="Actions" label-indent="false" align="end">
                        <div class="flex items-center justify-end gap-2">
                            <button
                                id="reset-button"
                                action="reset"
                                variant="secondary"
                                size="sm"
                            >
                                Reset
                            </button>
                            <button
                                id="save-button"
                                action="save"
                                data-record="profile"
                                variant="primary"
                                size="sm"
                            >
                                Save
                            </button>
                        </div>
                    </field>
                </form>
            </group-box>

            <group-box title="Bound feedback" variant="fill">
                <div class="flex flex-col gap-4">
                    <div class="flex items-center justify-between gap-4">
                        <div class="flex flex-col gap-2 flex-1">
                            <label
                                bind="completion_label"
                                secondary="Bound Progress value"
                            ></label>
                            <progress
                                id="profile-progress"
                                bind="completion"
                                size="sm"
                                class="w-full"
                            ></progress>
                        </div>
                        <badge
                            id="save-badge"
                            bind="save_count_value"
                            max="99"
                            size="lg"
                        >
                            <span class="p-4 bg-zinc-800 rounded-lg">
                                Committed saves
                            </span>
                        </badge>
                    </div>
                    <separator label="native feedback components" dashed></separator>
                    <div class="flex items-center gap-4">
                        <spinner id="showcase-spinner" size="sm"></spinner>
                        <span class="text-sm text-zinc-400">
                            Spinner and Skeleton retain native rendering behavior.
                        </span>
                    </div>
                    <skeleton class="w-full p-4 rounded-lg"></skeleton>
                </div>
            </group-box>

            <group-box title="Static table composed from native table primitives" variant="outline">
                <table size="sm" class="w-full border border-zinc-700 rounded-lg overflow-hidden">
                    <thead>
                        <tr>
                            <th>Name</th>
                            <th>Type</th>
                            <th align="center">Status</th>
                            <th>Updated</th>
                            <th align="right">Action</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td>Production PostgreSQL</td>
                            <td>PostgreSQL</td>
                            <td align="center"><tag variant="success" size="sm">Healthy</tag></td>
                            <td>2 minutes ago</td>
                            <td align="right">
                                <button
                                    action="inspect-row"
                                    data-connection="Production PostgreSQL"
                                    variant="ghost"
                                    size="xs"
                                >
                                    Inspect
                                </button>
                            </td>
                        </tr>
                        <tr>
                            <td>Analytics Replica</td>
                            <td>MySQL</td>
                            <td align="center"><tag variant="warning" size="sm">Lagging</tag></td>
                            <td>18 minutes ago</td>
                            <td align="right">
                                <button
                                    action="inspect-row"
                                    data-connection="Analytics Replica"
                                    variant="ghost"
                                    size="xs"
                                >
                                    Inspect
                                </button>
                            </td>
                        </tr>
                        <tr>
                            <td>Local Cache</td>
                            <td>Redis</td>
                            <td align="center"><tag variant="info" size="sm">Ready</tag></td>
                            <td>Just now</td>
                            <td align="right">
                                <button
                                    action="inspect-row"
                                    data-connection="Local Cache"
                                    variant="ghost"
                                    size="xs"
                                >
                                    Inspect
                                </button>
                            </td>
                        </tr>
                    </tbody>
                    <caption>
                        Table, sections, rows, heads, cells and caption are strongly structured.
                    </caption>
                </table>
            </group-box>

            <group-box title="Static declarative list with native ListItem rows" variant="outline">
                <div class="flex flex-col gap-2">
                    <label
                        bind="selected_connection"
                        secondary="Last action payload"
                    ></label>
                    <list class="gap-2">
                        <list-item
                            selected
                            action="select-connection"
                            data-connection="Production PostgreSQL"
                            class="border border-zinc-700 rounded-md"
                        >
                            Production PostgreSQL · selected
                        </list-item>
                        <list-item
                            confirmed
                            action="select-connection"
                            data-connection="Analytics Replica"
                            class="border border-zinc-700 rounded-md"
                        >
                            Analytics Replica · confirmed
                        </list-item>
                        <list-item
                            action="select-connection"
                            data-connection="Local Cache"
                            class="border border-zinc-700 rounded-md"
                        >
                            Local Cache · actionable
                        </list-item>
                        <list-item
                            disabled
                            class="border border-zinc-700 rounded-md"
                        >
                            Archived connection · disabled
                        </list-item>
                    </list>
                </div>
            </group-box>

            <group-box title="Display and navigation primitives" variant="outline">
                <div class="flex flex-col gap-4">
                    <div class="flex items-center justify-between gap-4">
                        <div class="flex items-center gap-4">
                            <avatar name="Ada Lovelace" size="lg"></avatar>
                            <div class="flex flex-col gap-2">
                                <span class="font-semibold">Native Avatar and AvatarGroup</span>
                                <span class="text-sm text-zinc-400">
                                    Initials are rendered without loading an external resource.
                                </span>
                            </div>
                        </div>
                        <avatar-group limit="3" ellipsis size="sm">
                            <avatar name="Ada Lovelace"></avatar>
                            <avatar name="Grace Hopper"></avatar>
                            <avatar name="Margaret Hamilton"></avatar>
                            <avatar name="Barbara Liskov"></avatar>
                        </avatar-group>
                    </div>

                    <description-list
                        layout="horizontal"
                        columns="3"
                        label-width="116"
                        size="sm"
                    >
                        <description-item label="Runtime">Rust actions</description-item>
                        <description-item label="State">String key/value store</description-item>
                        <description-item label="Rendering">gpui-component</description-item>
                        <description-item label="Contract" span="3">
                            Strict schemas, structural children, typed diagnostics, and stable IDs
                        </description-item>
                    </description-list>

                    <div class="flex items-center gap-2">
                        <span class="text-sm font-semibold">Save shortcut</span>
                        <kbd stroke="cmd-enter" outline></kbd>
                    </div>

                    <separator label="action-only breadcrumb"></separator>
                    <breadcrumb>
                        <breadcrumb-item
                            action="navigate"
                            data-destination="Workspace"
                        >
                            Workspace
                        </breadcrumb-item>
                        <breadcrumb-item
                            action="navigate"
                            data-destination="Connections"
                        >
                            Connections
                        </breadcrumb-item>
                        <breadcrumb-item disabled>Production PostgreSQL</breadcrumb-item>
                    </breadcrumb>

                    <label
                        bind="navigation_status"
                        secondary="Bindings commit before selection-changed actions run"
                    ></label>

                    <div class="flex flex-col gap-2">
                        <span class="text-sm font-semibold">Bound Pagination</span>
                        <pagination
                            id="showcase-pagination"
                            bind="current_page"
                            total-pages="12"
                            visible-pages="7"
                            size="sm"
                            action="selection-changed"
                            data-control="pagination"
                        ></pagination>
                    </div>

                    <div class="flex flex-col gap-2">
                        <span class="text-sm font-semibold">Bound Rating</span>
                        <rating
                            id="showcase-rating"
                            bind="rating"
                            max="5"
                            size="sm"
                            action="selection-changed"
                            data-control="rating"
                        ></rating>
                    </div>

                    <div class="flex flex-col gap-2">
                        <span class="text-sm font-semibold">Bound Slider</span>
                        <slider
                            id="showcase-slider"
                            bind="volume"
                            min="0"
                            max="100"
                            step="1"
                            orientation="horizontal"
                            scale="linear"
                            action="selection-changed"
                            data-control="slider"
                            class="w-full"
                        ></slider>
                    </div>

                    <div class="flex flex-col gap-2">
                        <span class="text-sm font-semibold">Bound Tabs</span>
                        <tabs
                            id="showcase-tabs"
                            bind="selected_tab"
                            variant="underline"
                            size="sm"
                            action="selection-changed"
                            data-control="tabs"
                        >
                            <tab>Overview</tab>
                            <tab>Activity</tab>
                            <tab disabled>Audit (disabled)</tab>
                        </tabs>
                    </div>

                    <div class="flex flex-col gap-2">
                        <span class="text-sm font-semibold">Bound Stepper</span>
                        <stepper
                            id="showcase-stepper"
                            bind="selected_step"
                            size="sm"
                            action="selection-changed"
                            data-control="stepper"
                        >
                            <stepper-item>
                                <span>Configure</span>
                            </stepper-item>
                            <stepper-item>
                                <span>Review</span>
                            </stepper-item>
                            <stepper-item>
                                <span>Apply</span>
                            </stepper-item>
                        </stepper>
                    </div>
                </div>
            </group-box>

            <group-box
                title="Controlled disclosure, resizable, and scroll layout"
                variant="outline"
            >
                <div class="flex flex-col gap-4">
                    <div class="flex flex-col gap-2">
                        <span class="font-semibold">Bound Accordion</span>
                        <span class="text-sm text-zinc-400">
                            Open indices use canonical JSON; the binding is committed before
                            the Action runs.
                        </span>
                        <accordion
                            id="showcase-accordion"
                            bind="open_sections"
                            multiple
                            size="sm"
                            action="accordion-changed"
                            data-control="accordion"
                            class="w-full"
                        >
                            <accordion-item title="General" class="p-2">
                                <description-list columns="2" size="sm">
                                    <description-item label="State key">
                                        open_sections
                                    </description-item>
                                    <description-item label="Encoding">
                                        JSON indices
                                    </description-item>
                                </description-list>
                            </accordion-item>
                            <accordion-item title="Advanced">
                                <tag variant="info" size="sm">
                                    Multiple sections may remain open
                                </tag>
                            </accordion-item>
                            <accordion-item title="Runtime contract">
                                <span class="text-sm text-zinc-400">
                                    Action handlers observe the newly committed binding.
                                </span>
                            </accordion-item>
                        </accordion>
                    </div>

                    <separator label="controlled collapsible"></separator>
                    <collapsible id="showcase-collapsible" bind="details_open" class="gap-2">
                        <div class="flex items-center justify-between gap-4">
                            <div class="flex flex-col gap-2">
                                <span class="font-semibold">Bound Collapsible</span>
                                <span class="text-sm text-zinc-400">
                                    The native component is controlled; a Runtime action changes
                                    its open binding.
                                </span>
                            </div>
                            <button
                                action="toggle-details"
                                variant="secondary"
                                size="sm"
                            >
                                Toggle details
                            </button>
                        </div>
                        <collapsible-content
                            class="p-4 bg-zinc-900 rounded-lg border border-zinc-700"
                        >
                            <description-list columns="2" size="sm">
                                <description-item label="State key">details_open</description-item>
                                <description-item label="Native input">open(bool)</description-item>
                            </description-list>
                        </collapsible-content>
                    </collapsible>

                    <separator label="drag the native resize handle"></separator>
                    <span class="text-sm text-zinc-400">
                        Resizable keeps panel sizes in GPUI window-keyed state under the
                        declaration's stable id.
                    </span>
                    <resizable
                        id="showcase-resizable"
                        orientation="horizontal"
                        size="220"
                        class="w-full border border-zinc-700 rounded-lg"
                    >
                        <resizable-panel
                            size="280"
                            min-size="140"
                            max-size="480"
                            class="p-4 bg-zinc-900"
                        >
                            <div class="flex flex-col gap-2">
                                <span class="font-semibold">Navigation panel</span>
                                <span class="text-sm text-zinc-400">
                                    Initial 280 px · range 140–480 px
                                </span>
                            </div>
                        </resizable-panel>
                        <resizable-panel min-size="180" class="p-4 bg-zinc-800">
                            <div class="flex flex-col gap-2">
                                <span class="font-semibold">Content panel</span>
                                <span class="text-sm text-zinc-400">
                                    Native drag behavior redistributes space between siblings.
                                </span>
                            </div>
                        </resizable-panel>
                    </resizable>

                    <separator label="native scroll handle and scrollbar"></separator>
                    <span class="text-sm text-zinc-400">
                        Scroll owns a stable native handle keyed by its explicit id. The
                        viewport height is finite while width comes from the parent layout.
                    </span>
                    <scroll
                        id="showcase-audit-log"
                        axis="vertical"
                        scrollbar-show="always"
                        height="180"
                        class="w-full border border-zinc-700 rounded-lg bg-zinc-900"
                    >
                        <div class="flex flex-col gap-2 p-4">
                            <span class="font-semibold">Audit log</span>
                            <span>09:41 Connected to production PostgreSQL</span>
                            <span>09:42 Refreshed public schema metadata</span>
                            <span>09:43 Loaded 48 relation definitions</span>
                            <span>09:44 Prepared migration preview</span>
                            <span>09:45 Validated role permissions</span>
                            <span>09:46 Exported diagnostics bundle</span>
                            <span>09:47 Reconciled declarative runtime state</span>
                            <span>09:48 Preserved ScrollHandle state after rerender</span>
                        </div>
                    </scroll>
                </div>
            </group-box>

            <separator label="registry extension point"></separator>
            <sql-editor
                id="sql-editor"
                class="p-4 bg-zinc-900 rounded-lg border border-zinc-700"
            />
        </div>
    </main>
</div>
"#;

fn main() {
    let mut registry = ComponentRegistry::with_defaults();
    if let Err(error) = registry.register("sql-editor", SqlEditorComponent) {
        eprintln!("failed to register demo component: {error}");
        return;
    }
    let template = match compile_template(DEMO_HTML, &registry, CompileOptions::strict()) {
        Ok(template) => template,
        Err(error) => {
            eprintln!("failed to compile demo template: {error}");
            return;
        }
    };

    gpui_platform::application()
        .with_assets(Assets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx| {
            gpui_component::init(cx);
            let runtime = cx.new(|_| demo_runtime());
            let bounds =
                Bounds::centered(None, size(px(WINDOW_WIDTH_PX), px(WINDOW_HEIGHT_PX)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Declarative UI Standalone v1".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(WINDOW_MIN_WIDTH_PX), px(WINDOW_MIN_HEIGHT_PX))),
                ..Default::default()
            };

            if let Err(error) = cx.open_window(options, move |window, cx| {
                window.activate_window();
                let view = cx.new(|cx| {
                    let config = DeclarativeViewConfig::new(
                        template.clone(),
                        runtime.clone(),
                        registry.clone(),
                    );
                    DeclarativeView::new(config, cx)
                });
                cx.new(|cx| Root::new(view, window, cx))
            }) {
                eprintln!("failed to open declarative UI demo window: {error}");
            }
        });
}

fn demo_runtime() -> Runtime {
    let mut state = StateStore::default();
    state.set("username", "admin");
    state.set("email", "admin@example.com");
    state.set(
        "notes",
        "Review replication health before the maintenance window.",
    );
    state.set("notifications", "true");
    state.set("auto_sync", "true");
    state.set("beta_mode", "false");
    state.set(
        "status_message",
        "Runtime ready. Edit fields, toggle controls, or dispatch an action.",
    );
    state.set("save_count_value", "0");
    state.set("completion", "38");
    state.set("completion_label", "Profile completeness: 38%");
    state.set(
        "selected_connection",
        "No connection action dispatched yet.",
    );
    state.set("current_page", "3");
    state.set("rating", "4");
    state.set("volume", "35");
    state.set("selected_tab", "1");
    state.set("selected_step", "1");
    state.set("open_sections", "[0]");
    state.set("details_open", "true");
    state.set(
        "navigation_status",
        "Pagination=3 · Rating=4 · Slider=35 · Tab=1 · Step=1",
    );
    let mut runtime = Runtime::new(state);
    runtime
        .on("save", |context| {
            let username = context.get("username").unwrap_or_default().to_owned();
            let notifications = context.get("notifications").unwrap_or("false").to_owned();
            let auto_sync = context.get("auto_sync").unwrap_or("false").to_owned();
            let count = context
                .get("save_count_value")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_default()
                + 1;
            let completion = context
                .get("completion")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or_default()
                .saturating_add(17)
                .min(100);
            context.set("save_count_value", count.to_string());
            context.set("completion", completion.to_string());
            context.set(
                "completion_label",
                format!("Profile completeness: {completion}%"),
            );
            context.set(
                "status_message",
                format!("Saved {username}; notifications={notifications}, auto-sync={auto_sync}."),
            );
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
        .on("reset", |context| {
            context.set("username", "admin");
            context.set("email", "admin@example.com");
            context.set(
                "notes",
                "Review replication health before the maintenance window.",
            );
            context.set("notifications", "true");
            context.set("auto_sync", "true");
            context.set("beta_mode", "false");
            context.set("save_count_value", "0");
            context.set("completion", "38");
            context.set("completion_label", "Profile completeness: 38%");
            context.set("current_page", "3");
            context.set("rating", "4");
            context.set("volume", "35");
            context.set("selected_tab", "1");
            context.set("selected_step", "1");
            context.set("open_sections", "[0]");
            context.set("details_open", "true");
            context.set(
                "navigation_status",
                "Pagination=3 · Rating=4 · Slider=35 · Tab=1 · Step=1",
            );
            context.set(
                "status_message",
                "State reset through one transactional Rust action.",
            );
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
        .on("accordion-changed", |context| {
            let sections = context.get("open_sections").unwrap_or("[]").to_owned();
            context.set(
                "status_message",
                format!("Accordion open sections changed to {sections}."),
            );
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
        .on("toggle-details", |context| {
            let open = context
                .get("details_open")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
            context.set("details_open", (!open).to_string());
            context.set(
                "status_message",
                format!(
                    "Collapsible is now {} through a transactional Runtime action.",
                    if open { "closed" } else { "open" }
                ),
            );
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
        .on("inspect-row", |context| {
            let connection = context
                .event()
                .payload()
                .get("connection")
                .cloned()
                .unwrap_or_else(|| "unknown connection".to_owned());
            context.set(
                "status_message",
                format!("Inspect action received table payload: {connection}."),
            );
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
        .on("select-connection", |context| {
            let connection = context
                .event()
                .payload()
                .get("connection")
                .cloned()
                .unwrap_or_else(|| "unknown connection".to_owned());
            context.set(
                "selected_connection",
                format!("Selected via ListItem action: {connection}"),
            );
            context.set(
                "status_message",
                format!("List action received payload: {connection}."),
            );
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
        .on("selection-changed", |context| {
            let control = context
                .event()
                .payload()
                .get("control")
                .map(String::as_str)
                .unwrap_or("unknown");
            let binding = match control {
                "pagination" => "current_page",
                "rating" => "rating",
                "slider" => "volume",
                "tabs" => "selected_tab",
                "stepper" => "selected_step",
                _ => {
                    context.set(
                        "navigation_status",
                        format!("Unknown selection control `{control}`."),
                    );
                    return Ok(());
                }
            };
            let value = context.get(binding).unwrap_or_default().to_owned();
            context.set(
                "navigation_status",
                format!(
                    "{control} selected {value}; its binding was committed before this action."
                ),
            );
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
        .on("navigate", |context| {
            let destination = context
                .event()
                .payload()
                .get("destination")
                .cloned()
                .unwrap_or_else(|| "unknown destination".to_owned());
            context.set(
                "navigation_status",
                format!("Breadcrumb action requested `{destination}` (no URL capability used)."),
            );
            Ok(())
        })
        .expect("demo action declarations must be unique");
    runtime
}

struct SqlEditorComponent;

impl ComponentRenderer for SqlEditorComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let element = div()
            .flex()
            .flex_col()
            .gap(px(SQL_EDITOR_GAP_PX))
            .child(div().text_color(rgb(MUTED_TEXT_RGB)).child("SQL Editor"))
            .child(
                div()
                    .text_color(rgb(PRIMARY_TEXT_RGB))
                    .child("SELECT * FROM connections;"),
            );
        Ok(context.style(element, &props).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{DEMO_HTML, SqlEditorComponent};
    use declarative_ui_demo::{CompileOptions, ComponentRegistry, compile_template};

    #[test]
    fn showcase_template_satisfies_the_strict_dsl_contract() {
        let mut registry = ComponentRegistry::with_defaults();
        registry
            .register("sql-editor", SqlEditorComponent)
            .expect("register showcase extension");

        compile_template(DEMO_HTML, &registry, CompileOptions::strict())
            .expect("the shipped showcase must compile in strict mode");
    }
}
