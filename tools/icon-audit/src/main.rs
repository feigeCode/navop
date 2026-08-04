use anyhow::{Context as _, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use syn::{Expr, ImplItem, Item, Pat, Stmt, Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Finding {
    severity: Severity,
    code: &'static str,
    subject: String,
    message: String,
}

impl Finding {
    fn error(code: &'static str, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            subject: subject.into(),
            message: message.into(),
        }
    }

    fn warning(code: &'static str, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            subject: subject.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Default)]
struct ParsedIcons {
    variants: BTreeSet<String>,
    variant_order: Vec<String>,
    all_order: Option<Vec<String>>,
    paths: BTreeMap<String, String>,
    kinds: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct AuditReport {
    icon_count: usize,
    asset_count: usize,
    findings: Vec<Finding>,
}

impl AuditReport {
    fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .count()
    }

    fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Warning)
            .count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Check,
    Gallery,
}

#[derive(Debug)]
struct Args {
    command: Command,
    workspace: Option<PathBuf>,
    deny_warnings: bool,
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let workspace = match args.workspace {
        Some(path) => path,
        None => find_workspace_root(&env::current_dir().context("resolve current directory")?)?,
    };
    let icon_source_path = workspace.join("crates/ui/src/icon.rs");
    let icon_metadata_path = workspace.join("crates/ui/src/icon/metadata.rs");
    let assets_root = workspace.join("crates/assets/assets");
    let icon_source = fs::read_to_string(&icon_source_path)
        .with_context(|| format!("read {}", icon_source_path.display()))?;
    let icon_metadata = fs::read_to_string(&icon_metadata_path)
        .with_context(|| format!("read {}", icon_metadata_path.display()))?;
    let source = format!("{icon_source}\n{icon_metadata}");
    let parsed = parse_icon_source(&source)?;

    match args.command {
        Command::Gallery => {
            print_gallery_inventory(&parsed);
            Ok(())
        }
        Command::Check => {
            let report = audit_icons(&parsed, &assets_root)?;
            print_report(&report);
            if report.error_count() > 0 {
                bail!("icon audit failed with {} error(s)", report.error_count());
            }
            if args.deny_warnings && report.warning_count() > 0 {
                bail!(
                    "icon audit failed because --deny-warnings found {} warning(s)",
                    report.warning_count()
                );
            }
            Ok(())
        }
    }
}

fn parse_args() -> Result<Args> {
    let mut command = Command::Check;
    let mut workspace = None;
    let mut deny_warnings = false;
    let mut positional_seen = false;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "check" | "lint" => {
                if positional_seen {
                    bail!("only one command may be supplied");
                }
                command = Command::Check;
                positional_seen = true;
            }
            "gallery" => {
                if positional_seen {
                    bail!("only one command may be supplied");
                }
                command = Command::Gallery;
                positional_seen = true;
            }
            "--workspace" => {
                let value = args.next().context("--workspace requires a path")?;
                workspace = Some(PathBuf::from(value));
            }
            "--deny-warnings" => deny_warnings = true,
            "-h" | "--help" => {
                println!(
                    "Usage: icon-audit [check|lint|gallery] [--workspace PATH] [--deny-warnings]\n\
                     \n\
                     check/lint  validate IconName mappings and SVG structure\n\
                     gallery     print a tab-separated icon inventory"
                );
                std::process::exit(0);
            }
            unknown => bail!("unknown argument: {unknown}"),
        }
    }

    Ok(Args {
        command,
        workspace,
        deny_warnings,
    })
}

fn find_workspace_root(start: &Path) -> Result<PathBuf> {
    for candidate in start.ancestors() {
        let manifest = candidate.join("Cargo.toml");
        if let Ok(contents) = fs::read_to_string(&manifest)
            && contents.contains("[workspace]")
            && candidate.join("crates/ui/src/icon.rs").is_file()
        {
            return Ok(candidate.to_path_buf());
        }
    }
    bail!(
        "could not find the Navop workspace root from {}",
        start.display()
    )
}

fn parse_icon_source(source: &str) -> Result<ParsedIcons> {
    let file = syn::parse_file(source).context("parse crates/ui/src/icon.rs")?;
    let mut parsed = ParsedIcons::default();
    let mut path_match = None;
    let mut kind_match = None;

    for item in &file.items {
        match item {
            Item::Enum(item_enum) if item_enum.ident == "IconName" => {
                parsed.variant_order = item_enum
                    .variants
                    .iter()
                    .map(|variant| variant.ident.to_string())
                    .collect();
                parsed.variants.extend(parsed.variant_order.iter().cloned());
            }
            Item::Impl(item_impl)
                if trait_name(item_impl).is_some_and(|name| name == "IconNamed")
                    && self_type_name(item_impl).is_some_and(|name| name == "IconName") =>
            {
                path_match = find_method_match(item_impl, "path");
            }
            Item::Impl(item_impl)
                if item_impl.trait_.is_none()
                    && self_type_name(item_impl).is_some_and(|name| name == "IconName") =>
            {
                kind_match = find_method_match(item_impl, "kind").or(kind_match);
                if let Some(all_order) = find_icon_all(item_impl)? {
                    if parsed.all_order.replace(all_order).is_some() {
                        bail!("IconName::ALL is defined more than once");
                    }
                }
            }
            _ => {}
        }
    }

    if parsed.variants.is_empty() {
        bail!("IconName enum was not found or has no variants");
    }

    let path_match = path_match.context("IconNamed::path match expression was not found")?;
    for arm in &path_match.arms {
        let names = pattern_variant_names(&arm.pat)?;
        let path = expression_string(&arm.body)
            .with_context(|| format!("path arm for {names:?} must return a string literal"))?;
        for name in names {
            if parsed.paths.insert(name.clone(), path.clone()).is_some() {
                bail!("duplicate IconName::path mapping for {name}");
            }
        }
    }

    let kind_match = kind_match.context("IconName::kind match expression was not found")?;
    for arm in &kind_match.arms {
        let kind = expression_ident(&arm.body)
            .context("IconName::kind arm must return an IconKind variant")?;
        for name in pattern_variant_names(&arm.pat)? {
            if parsed.kinds.insert(name.clone(), kind.clone()).is_some() {
                bail!("duplicate IconName::kind classification for {name}");
            }
        }
    }

    Ok(parsed)
}

fn find_icon_all(item_impl: &syn::ItemImpl) -> Result<Option<Vec<String>>> {
    let Some(constant) = item_impl.items.iter().find_map(|item| {
        let ImplItem::Const(constant) = item else {
            return None;
        };
        (constant.ident == "ALL").then_some(constant)
    }) else {
        return Ok(None);
    };

    let array = expression_array(&constant.expr)
        .context("IconName::ALL must be a reference to an array of IconName variants")?;
    let mut variants = Vec::with_capacity(array.elems.len());
    for expression in &array.elems {
        variants.push(
            expression_ident(expression)
                .context("IconName::ALL entries must be IconName variant paths")?,
        );
    }
    Ok(Some(variants))
}

fn expression_array(expression: &Expr) -> Option<&syn::ExprArray> {
    match expression {
        Expr::Array(array) => Some(array),
        Expr::Reference(reference) => expression_array(&reference.expr),
        Expr::Paren(paren) => expression_array(&paren.expr),
        Expr::Group(group) => expression_array(&group.expr),
        _ => None,
    }
}

fn trait_name(item_impl: &syn::ItemImpl) -> Option<&syn::Ident> {
    item_impl
        .trait_
        .as_ref()?
        .1
        .segments
        .last()
        .map(|segment| &segment.ident)
}

fn self_type_name(item_impl: &syn::ItemImpl) -> Option<&syn::Ident> {
    let Type::Path(type_path) = item_impl.self_ty.as_ref() else {
        return None;
    };
    type_path.path.segments.last().map(|segment| &segment.ident)
}

fn find_method_match<'a>(
    item_impl: &'a syn::ItemImpl,
    method_name: &str,
) -> Option<&'a syn::ExprMatch> {
    let method = item_impl.items.iter().find_map(|item| {
        let ImplItem::Fn(method) = item else {
            return None;
        };
        (method.sig.ident == method_name).then_some(method)
    })?;
    method
        .block
        .stmts
        .iter()
        .find_map(statement_expression)
        .and_then(find_match_expression)
}

fn statement_expression(statement: &Stmt) -> Option<&Expr> {
    match statement {
        Stmt::Expr(expression, _) => Some(expression),
        Stmt::Local(local) => local.init.as_ref().map(|init| init.expr.as_ref()),
        _ => None,
    }
}

fn find_match_expression(expression: &Expr) -> Option<&syn::ExprMatch> {
    match expression {
        Expr::Match(expr_match) => Some(expr_match),
        Expr::Block(block) => block
            .block
            .stmts
            .iter()
            .find_map(statement_expression)
            .and_then(find_match_expression),
        Expr::Paren(paren) => find_match_expression(&paren.expr),
        Expr::Group(group) => find_match_expression(&group.expr),
        Expr::MethodCall(method_call) => find_match_expression(&method_call.receiver),
        _ => None,
    }
}

fn pattern_variant_names(pattern: &Pat) -> Result<Vec<String>> {
    match pattern {
        Pat::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| vec![segment.ident.to_string()])
            .context("empty path in IconName match pattern"),
        Pat::Or(or_pattern) => {
            let mut names = Vec::new();
            for case in &or_pattern.cases {
                names.extend(pattern_variant_names(case)?);
            }
            Ok(names)
        }
        Pat::Wild(_) => bail!("IconName::kind must classify every variant explicitly"),
        _ => bail!("unsupported IconName match pattern"),
    }
}

fn expression_string(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Lit(literal) => {
            let syn::Lit::Str(value) = &literal.lit else {
                return None;
            };
            Some(value.value())
        }
        Expr::Paren(paren) => expression_string(&paren.expr),
        Expr::Group(group) => expression_string(&group.expr),
        _ => None,
    }
}

fn expression_ident(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        Expr::Paren(paren) => expression_ident(&paren.expr),
        Expr::Group(group) => expression_ident(&group.expr),
        _ => None,
    }
}

fn audit_icons(parsed: &ParsedIcons, assets_root: &Path) -> Result<AuditReport> {
    let mut report = AuditReport {
        icon_count: parsed.variants.len(),
        ..AuditReport::default()
    };
    let assets = collect_svg_assets(assets_root)?;
    report.asset_count = assets.len();

    audit_all_registry(parsed, &mut report.findings);

    for variant in &parsed.variants {
        if !parsed.paths.contains_key(variant) {
            report.findings.push(Finding::error(
                "missing-path",
                variant,
                "IconName variant has no IconNamed::path mapping",
            ));
        }
        if !parsed.kinds.contains_key(variant) {
            report.findings.push(Finding::error(
                "missing-kind",
                variant,
                "IconName variant has no semantic IconKind",
            ));
        }
    }

    for variant in parsed.paths.keys() {
        if !parsed.variants.contains(variant) {
            report.findings.push(Finding::error(
                "unknown-variant",
                variant,
                "IconNamed::path maps a name not present in IconName",
            ));
        }
    }

    let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (variant, path) in &parsed.paths {
        owners.entry(path).or_default().push(variant);
        if let Some(message) = validate_canonical_path(path) {
            report.findings.push(Finding::error(
                "invalid-path",
                variant,
                format!("{path}: {message}"),
            ));
        }
        if !assets.contains_key(path) {
            report.findings.push(Finding::error(
                "missing-asset",
                variant,
                format!("{path} does not exist under {}", assets_root.display()),
            ));
        }
    }

    for (path, variants) in owners {
        if variants.len() > 1 {
            report.findings.push(Finding::error(
                "duplicate-path",
                path,
                format!("mapped by {}", variants.join(", ")),
            ));
        }
    }

    let mapped_paths: BTreeSet<&str> = parsed.paths.values().map(String::as_str).collect();
    for (path, contents) in &assets {
        if !mapped_paths.contains(path.as_str()) {
            report.findings.push(Finding::warning(
                "unmapped-asset",
                path,
                "SVG exists on disk but is not exposed by IconName",
            ));
        }
        audit_svg(
            path,
            contents,
            icon_kind_for_path(parsed, path),
            &mut report.findings,
        );
    }

    report.findings.sort_by(|a, b| {
        (a.severity as u8, &a.code, &a.subject).cmp(&(b.severity as u8, &b.code, &b.subject))
    });
    Ok(report)
}

fn audit_all_registry(parsed: &ParsedIcons, findings: &mut Vec<Finding>) {
    let Some(all_order) = &parsed.all_order else {
        findings.push(Finding::error(
            "missing-all",
            "IconName::ALL",
            "stable icon registry is not defined",
        ));
        return;
    };

    let mut seen = BTreeSet::new();
    for variant in all_order {
        if !seen.insert(variant) {
            findings.push(Finding::error(
                "all-duplicate-variant",
                variant,
                "IconName::ALL contains this variant more than once",
            ));
        }
        if !parsed.variants.contains(variant) {
            findings.push(Finding::error(
                "all-unknown-variant",
                variant,
                "IconName::ALL contains a name not present in IconName",
            ));
        }
    }

    for variant in &parsed.variant_order {
        if !seen.contains(variant) {
            findings.push(Finding::error(
                "all-missing-variant",
                variant,
                "IconName variant is missing from IconName::ALL",
            ));
        }
    }

    if all_order != &parsed.variant_order {
        findings.push(Finding::error(
            "all-order-mismatch",
            "IconName::ALL",
            format!(
                "registry order/length differs from enum declaration (ALL {}, enum {})",
                all_order.len(),
                parsed.variant_order.len()
            ),
        ));
    }
}

fn validate_canonical_path(path: &str) -> Option<&'static str> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Some("absolute paths are not allowed");
    }
    if !path.starts_with("icons/") || !path.ends_with(".svg") {
        return Some("canonical paths must match icons/**/*.svg");
    }
    if candidate
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Some("path traversal and special path components are not allowed");
    }
    None
}

fn collect_svg_assets(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut assets = BTreeMap::new();
    collect_svg_assets_recursive(root, root, &mut assets)?;
    Ok(assets)
}

fn collect_svg_assets_recursive(
    root: &Path,
    current: &Path,
    assets: &mut BTreeMap<String, String>,
) -> Result<()> {
    for entry in fs::read_dir(current).with_context(|| format!("read {}", current.display()))? {
        let entry = entry.with_context(|| format!("read entry under {}", current.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("read type for {}", path.display()))?;
        if file_type.is_dir() {
            collect_svg_assets_recursive(root, &path, assets)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("svg") {
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("relativize {}", path.display()))?;
            let key = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("read UTF-8 {}", path.display()))?;
            assets.insert(key, contents);
        }
    }
    Ok(())
}

fn icon_kind_for_path<'a>(parsed: &'a ParsedIcons, path: &str) -> Option<&'a str> {
    parsed
        .paths
        .iter()
        .find_map(|(variant, mapped_path)| {
            (mapped_path == path).then(|| parsed.kinds.get(variant).map(String::as_str))
        })
        .flatten()
}

fn audit_svg(path: &str, contents: &str, kind: Option<&str>, findings: &mut Vec<Finding>) {
    let parse_contents = strip_doctype(contents);
    let document = match roxmltree::Document::parse(&parse_contents) {
        Ok(document) => document,
        Err(error) => {
            findings.push(Finding::error(
                "invalid-svg",
                path,
                format!("XML parse failed: {error}"),
            ));
            return;
        }
    };
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        findings.push(Finding::error(
            "invalid-root",
            path,
            format!(
                "root element is <{}>, expected <svg>",
                root.tag_name().name()
            ),
        ));
        return;
    }

    match root.attribute("viewBox") {
        None => findings.push(Finding::error(
            "missing-viewbox",
            path,
            "root <svg> has no viewBox",
        )),
        Some(view_box) => match parse_view_box(view_box) {
            Some((_, _, width, height)) if width > 0.0 && height > 0.0 => {}
            Some(_) => findings.push(Finding::error(
                "invalid-viewbox",
                path,
                format!("viewBox has non-positive dimensions: {view_box}"),
            )),
            None => findings.push(Finding::error(
                "invalid-viewbox",
                path,
                format!("viewBox must contain four finite numbers: {view_box}"),
            )),
        },
    }

    if root.tag_name().namespace() != Some("http://www.w3.org/2000/svg") {
        findings.push(Finding::warning(
            "missing-xmlns",
            path,
            "root <svg> has no xmlns attribute",
        ));
    }

    let width = root.attribute("width");
    let height = root.attribute("height");
    if width.is_none() || height.is_none() {
        findings.push(Finding::warning(
            "missing-dimensions",
            path,
            "width and height should both be declared for predictable tooling previews",
        ));
    } else if let (Some(view_box), Some(width), Some(height)) =
        (root.attribute("viewBox"), width, height)
        && let (Some((_, _, view_width, view_height)), Some(width), Some(height)) = (
            parse_view_box(view_box),
            parse_dimension(width),
            parse_dimension(height),
        )
        && ((width - view_width).abs() > 0.01 || (height - view_height).abs() > 0.01)
    {
        findings.push(Finding::warning(
            "dimension-mismatch",
            path,
            format!(
                "width/height ({width}×{height}) differ from viewBox ({view_width}×{view_height})"
            ),
        ));
    }

    if contents.trim_start().starts_with("<?xml") || contents.contains("<!DOCTYPE") {
        findings.push(Finding::warning(
            "generated-preamble",
            path,
            "XML declaration or DOCTYPE should be removed from normalized assets",
        ));
    }

    if document.descendants().any(|node| {
        node.attributes().any(|attribute| {
            let name = attribute.name().to_ascii_lowercase();
            name.contains("generator") || name.starts_with("inkscape:")
        })
    }) {
        findings.push(Finding::warning(
            "generator-metadata",
            path,
            "generator-specific attributes should be removed when the asset is normalized",
        ));
    }

    if matches!(kind, Some("FunctionalOutline" | "FunctionalFilled"))
        && document.descendants().any(|node| {
            node.attributes().any(|attribute| {
                matches!(attribute.name(), "fill" | "stroke") && is_fixed_color(attribute.value())
            })
        })
    {
        findings.push(Finding::warning(
            "fixed-functional-color",
            path,
            "functional icon contains a fixed fill/stroke instead of currentColor",
        ));
    }
}

fn strip_doctype(contents: &str) -> String {
    let Some(start) = contents.find("<!DOCTYPE") else {
        return contents.to_owned();
    };
    let remainder = &contents[start..];
    let end = if let Some(internal_subset) = remainder.find('[') {
        remainder[internal_subset..]
            .find("]>")
            .map(|offset| internal_subset + offset + 2)
    } else {
        remainder.find('>').map(|offset| offset + 1)
    };
    let Some(end) = end else {
        return contents.to_owned();
    };
    let mut normalized = String::with_capacity(contents.len() - end);
    normalized.push_str(&contents[..start]);
    normalized.push_str(&contents[start + end..]);
    normalized
}

fn parse_view_box(value: &str) -> Option<(f64, f64, f64, f64)> {
    let values = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() != 4 || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    Some((values[0], values[1], values[2], values[3]))
}

fn parse_dimension(value: &str) -> Option<f64> {
    let value = value.trim().strip_suffix("px").unwrap_or(value.trim());
    let number = value.parse::<f64>().ok()?;
    number.is_finite().then_some(number)
}

fn is_fixed_color(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    !matches!(
        normalized.as_str(),
        "" | "none" | "currentcolor" | "inherit" | "transparent"
    ) && !normalized.starts_with("url(")
}

fn print_gallery_inventory(parsed: &ParsedIcons) {
    println!("name\tkind\tpath");
    let variants = parsed
        .all_order
        .as_deref()
        .unwrap_or(parsed.variant_order.as_slice());
    for variant in variants {
        let kind = parsed
            .kinds
            .get(variant)
            .map(String::as_str)
            .unwrap_or("Unknown");
        let path = parsed.paths.get(variant).map(String::as_str).unwrap_or("");
        println!("{variant}\t{kind}\t{path}");
    }
}

fn print_report(report: &AuditReport) {
    for finding in &report.findings {
        let label = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        println!(
            "{label}[{}] {}: {}",
            finding.code, finding.subject, finding.message
        );
    }
    println!(
        "icon-audit: {} IconName variants, {} SVG assets, {} error(s), {} warning(s)",
        report.icon_count,
        report.asset_count,
        report.error_count(),
        report.warning_count()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
        pub enum IconName { Add, Logo }
        pub trait IconNamed { fn path(self) -> String; }
        impl IconName {
            pub const ALL: &'static [Self] = &[Self::Add, Self::Logo];

            pub const fn kind(self) -> IconKind {
                use IconKind::{BrandColor, FunctionalOutline};
                match self {
                    Self::Add => FunctionalOutline,
                    Self::Logo => BrandColor,
                }
            }
        }
        impl IconNamed for IconName {
            fn path(self) -> String {
                match self {
                    Self::Add => "icons/add.svg".into(),
                    Self::Logo => "icons/logo.svg".into(),
                }
            }
        }
    "#;

    #[test]
    fn parses_icon_variants_paths_and_explicit_kinds() {
        let source = SOURCE.replace(".into()", "");
        let parsed = parse_icon_source(&source).expect("parse source");

        assert_eq!(parsed.variants.len(), 2);
        assert_eq!(parsed.variant_order, ["Add", "Logo"]);
        assert_eq!(
            parsed.all_order.as_deref(),
            Some(["Add".to_owned(), "Logo".to_owned()].as_slice())
        );
        assert_eq!(parsed.paths["Add"], "icons/add.svg");
        assert_eq!(parsed.kinds["Add"], "FunctionalOutline");
        assert_eq!(parsed.kinds["Logo"], "BrandColor");
    }

    #[test]
    fn reports_missing_explicit_kind_classification() {
        let source = SOURCE
            .replace("Self::Add => FunctionalOutline,", "")
            .replace(".into()", "");
        let parsed = parse_icon_source(&source).expect("parse source");
        let report = audit_icons(&parsed, Path::new("missing-assets"))
            .expect_err("asset directory is intentionally absent");

        assert!(report.to_string().contains("missing-assets"));
        let mut findings = Vec::new();
        for variant in &parsed.variants {
            if !parsed.kinds.contains_key(variant) {
                findings.push(Finding::error(
                    "missing-kind",
                    variant,
                    "IconName variant has no semantic IconKind",
                ));
            }
        }
        assert!(
            findings
                .iter()
                .any(|finding| { finding.code == "missing-kind" && finding.subject == "Add" })
        );
    }

    #[test]
    fn rejects_wildcard_kind_fallback() {
        let source = SOURCE
            .replace("Self::Add => FunctionalOutline,", "_ => FunctionalOutline,")
            .replace(".into()", "");

        let error = parse_icon_source(&source).expect_err("wildcard must be rejected");
        assert!(
            error
                .to_string()
                .contains("must classify every variant explicitly")
        );
    }

    #[test]
    fn reports_missing_all_registry() {
        let source = SOURCE
            .replace(
                "pub const ALL: &'static [Self] = &[Self::Add, Self::Logo];",
                "",
            )
            .replace(".into()", "");
        let parsed = parse_icon_source(&source).expect("parse source");
        let mut findings = Vec::new();

        audit_all_registry(&parsed, &mut findings);

        assert!(findings.iter().any(|finding| finding.code == "missing-all"));
    }

    #[test]
    fn reports_incomplete_duplicate_unknown_and_out_of_order_all_registry() {
        let source = SOURCE
            .replace(
                "&[Self::Add, Self::Logo]",
                "&[Self::Logo, Self::Logo, Self::Unknown]",
            )
            .replace(".into()", "");
        let parsed = parse_icon_source(&source).expect("parse source");
        let mut findings = Vec::new();

        audit_all_registry(&parsed, &mut findings);

        for code in [
            "all-duplicate-variant",
            "all-unknown-variant",
            "all-missing-variant",
            "all-order-mismatch",
        ] {
            assert!(
                findings.iter().any(|finding| finding.code == code),
                "missing finding {code}"
            );
        }
    }

    #[test]
    fn rejects_non_array_all_registry() {
        let source = SOURCE
            .replace("&[Self::Add, Self::Logo]", "Self::Add")
            .replace(".into()", "");

        assert!(parse_icon_source(&source).is_err());
    }

    #[test]
    fn validates_canonical_paths() {
        assert_eq!(validate_canonical_path("icons/add.svg"), None);
        assert!(validate_canonical_path("../add.svg").is_some());
        assert!(validate_canonical_path("/icons/add.svg").is_some());
        assert!(validate_canonical_path("assets/add.png").is_some());
    }

    #[test]
    fn parses_valid_view_boxes_and_dimensions() {
        assert_eq!(parse_view_box("0 0 24 24"), Some((0.0, 0.0, 24.0, 24.0)));
        assert_eq!(parse_view_box("0,0,16,20"), Some((0.0, 0.0, 16.0, 20.0)));
        assert_eq!(parse_view_box("0 0 24"), None);
        assert_eq!(parse_dimension("24px"), Some(24.0));
    }

    #[test]
    fn strips_external_doctype_before_xml_parsing() {
        let source = r#"<?xml version="1.0"?><!DOCTYPE svg PUBLIC "id" "url"><svg/>"#;

        assert_eq!(strip_doctype(source), r#"<?xml version="1.0"?><svg/>"#);
    }

    #[test]
    fn svg_audit_distinguishes_errors_and_warnings() {
        let mut findings = Vec::new();
        audit_svg(
            "icons/test.svg",
            r##"<svg viewBox="0 0 24 24"><path stroke="#000"/></svg>"##,
            Some("FunctionalOutline"),
            &mut findings,
        );

        assert!(
            findings
                .iter()
                .all(|finding| finding.severity == Severity::Warning)
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "fixed-functional-color")
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "missing-dimensions")
        );
    }
}
