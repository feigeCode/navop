import { readFileSync, readdirSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type DefaultTheme } from "vitepress";

const docsRoot = fileURLToPath(new URL("../", import.meta.url));

type LocaleOptions = {
  prefix: string;
  title: string;
  quickStart: string;
  productSite: string;
  releases: string;
  groups: [string, string, string, string, string];
  outline: string;
  lastUpdated: string;
  editPage: string;
  previous: string;
  next: string;
};

const guideOrder = [
  "quick-start",
  "install-update",
  "workspace-connections",
  "database-connections",
  "sql-editor",
  "table-data",
  "schema-tools",
  "database-transfer",
  "redis",
  "mongodb",
  "ssh-terminal",
  "sftp-remote-files",
  "port-forwarding",
  "remote-access",
  "notes",
  "ai-workbench",
  "public-mcp",
  "extensions",
  "teams-sync-security",
  "settings-shortcuts",
  "troubleshooting"
] as const;

const localeOptions: Record<string, LocaleOptions> = {
  root: {
    prefix: "",
    title: "Navop 使用说明",
    quickStart: "快速开始",
    productSite: "产品官网",
    releases: "下载桌面端",
    groups: ["开始使用", "数据库与数据", "远程工作", "笔记与 AI", "团队与设置"],
    outline: "本页目录",
    lastUpdated: "最后更新",
    editPage: "在 GitHub 上编辑此页",
    previous: "上一章",
    next: "下一章"
  },
  "zh-TW": {
    prefix: "/zh-TW",
    title: "Navop 使用說明",
    quickStart: "快速開始",
    productSite: "產品官網",
    releases: "下載桌面端",
    groups: ["開始使用", "資料庫與資料", "遠端工作", "筆記與 AI", "團隊與設定"],
    outline: "本頁目錄",
    lastUpdated: "最後更新",
    editPage: "在 GitHub 上編輯此頁",
    previous: "上一章",
    next: "下一章"
  },
  "en-US": {
    prefix: "/en-US",
    title: "Navop Usage Guide",
    quickStart: "Quick start",
    productSite: "Product website",
    releases: "Download desktop app",
    groups: ["Getting started", "Databases and data", "Remote work", "Notes and AI", "Teams and settings"],
    outline: "On this page",
    lastUpdated: "Last updated",
    editPage: "Edit this page on GitHub",
    previous: "Previous",
    next: "Next"
  }
};

function guideItems(options: LocaleOptions): DefaultTheme.SidebarItem[] {
  const guideDir = join(docsRoot, options.prefix.replace(/^\//, ""), "guide");
  const itemsBySlug = new Map(readdirSync(guideDir)
    .filter((file) => file.endsWith(".md"))
    .map((file) => {
      const title = readFileSync(join(guideDir, file), "utf8").match(/^#\s+(.+)$/m)?.[1]?.trim();
      const slug = basename(file, ".md");
      return [slug, {
        text: title || slug,
        link: `${options.prefix}/guide/${slug}`
      }] as const;
    }));
  const items = guideOrder.flatMap((slug) => {
    const item = itemsBySlug.get(slug);
    return item ? [item] : [];
  });

  const ranges: Array<[number, number]> = [
    [0, 3],
    [3, 10],
    [10, 14],
    [14, 18],
    [18, 21]
  ];

  return ranges.map(([start, end], index) => ({
    text: options.groups[index],
    collapsed: false,
    items: items.slice(start, end)
  }));
}

function localeTheme(options: LocaleOptions): DefaultTheme.LocaleConfig<DefaultTheme.Config> {
  return {
    nav: [
      { text: options.quickStart, link: `${options.prefix}/guide/quick-start` },
      { text: options.productSite, link: "https://navop.dev", target: "_blank", rel: "noopener" },
      { text: options.releases, link: "https://github.com/feigeCode/navop/releases", target: "_blank", rel: "noopener" }
    ],
    sidebar: guideItems(options),
    outline: { level: [2, 3], label: options.outline },
    docFooter: { prev: options.previous, next: options.next },
    lastUpdated: { text: options.lastUpdated },
    editLink: {
      pattern: "https://github.com/feigeCode/navop/edit/dev/docs-site/docs/:path",
      text: options.editPage
    }
  };
}

export default defineConfig({
  title: "Navop",
  description: "Navop 数据库、SSH、SFTP、终端、Notes、远程桌面与 AI 工作台使用说明。",
  cleanUrls: true,
  lastUpdated: true,
  lang: "zh-CN",
  locales: {
    root: {
      label: "简体中文",
      lang: "zh-CN",
      title: localeOptions.root.title,
      themeConfig: localeTheme(localeOptions.root)
    },
    "zh-TW": {
      label: "繁體中文",
      lang: "zh-TW",
      title: localeOptions["zh-TW"].title,
      themeConfig: localeTheme(localeOptions["zh-TW"])
    },
    "en-US": {
      label: "English",
      lang: "en-US",
      title: localeOptions["en-US"].title,
      themeConfig: localeTheme(localeOptions["en-US"])
    }
  },
  head: [
    ["meta", { name: "theme-color", content: "#0d6eaf" }],
    ["link", { rel: "icon", href: "/navop-icon.png" }]
  ],
  themeConfig: {
    logo: "/navop-icon.png",
    siteTitle: "Navop",
    socialLinks: [{ icon: "github", link: "https://github.com/feigeCode/navop" }],
    search: { provider: "local" },
    footer: {
      message: "Navop · Native workspace for data and remote operations",
      copyright: "Copyright © Navop"
    }
  }
});
