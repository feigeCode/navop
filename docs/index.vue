<template>
    <main class="home-page">
        <section class="hero">
            <div class="hero-copy">
                <p class="eyebrow">OnetCli / 桌面工作台</p>
                <h1>把数据库、SSH / SFTP 和终端收进一个真正顺手的桌面客户端</h1>
                <p class="hero-description">
                    OnetCli 是一个跨平台桌面工作台，用来统一处理数据库连接、远程访问、文件传输和本地终端操作。
                    AI 主要用于辅助数据分析、快速生成 SQL 和解释查询，帮助你更高效地完成日常工作。
                </p>
                <div class="hero-note">
                    <span>多数据库支持</span>
                    <span>原生 SSH / SFTP</span>
                    <span>内置终端</span>
                </div>
                <div class="hero-actions">
                    <a class="primary-action" href="https://github.com/feigeCode/onetcli/releases" target="_blank" rel="noreferrer">
                        <span>下载 OnetCli</span>
                    </a>
                    <a class="secondary-action" :href="withBase('/guide')">
                        <span>查看文档</span>
                    </a>
                </div>
                <dl class="hero-metrics">
                    <div>
                        <dt>聚焦任务</dt>
                        <dd>连接、查询、排查、传输、执行</dd>
                    </div>
                    <div>
                        <dt>适用平台</dt>
                        <dd>macOS、Windows、Linux</dd>
                    </div>
                </dl>
            </div>
            <div class="hero-visual">
                <img :src="heroImage" alt="OnetCli 应用主界面截图" />
            </div>
        </section>

        <section class="section-block screenshots">
            <div class="section-heading">
                <p class="section-label">真实界面</p>
                <h2>直接展示你每天会用到的工作界面</h2>
                <p>
                    不是一张抽象插画，而是数据库管理、SSH 会话、SFTP 文件传输和查询辅助的真实桌面界面。
                </p>
            </div>
            <div class="screenshot-grid">
                <article class="screenshot-card">
                    <img :src="databaseImage" alt="OnetCli 数据库界面截图" />
                    <h3>多数据库管理</h3>
                    <p>在同一个客户端里处理 PostgreSQL、MySQL、SQLite、SQL Server、Oracle、ClickHouse。</p>
                </article>
                <article class="screenshot-card">
                    <img :src="sshImage" alt="OnetCli SSH 与 SFTP 界面截图" />
                    <h3>SSH 与 SFTP</h3>
                    <p>从远程终端到文件传输，用一套连接信息覆盖排查、部署和日常维护。</p>
                </article>
                <article class="screenshot-card">
                    <img :src="chatdbImage" alt="OnetCli 查询与分析辅助界面截图" />
                    <h3>查询与分析辅助</h3>
                    <p>用自然语言辅助生成 SQL、解释查询结果，并在数据分析场景里提升处理效率。</p>
                </article>
            </div>
        </section>

        <section class="section-block capability-section">
            <div class="section-heading">
                <p class="section-label">核心能力</p>
                <h2>把数据库、远程连接和终端收成一个工作台</h2>
            </div>
            <div class="capability-grid">
                <article class="capability-card">
                    <h3>数据库</h3>
                    <p>在同一界面管理多种数据库连接，查看数据、执行 SQL、浏览对象结构。</p>
                </article>
                <article class="capability-card">
                    <h3>SSH / SFTP</h3>
                    <p>一套连接信息同时覆盖远程终端与文件管理，让排查和运维动作保持连贯。</p>
                </article>
                <article class="capability-card">
                    <h3>本地终端</h3>
                    <p>内置终端支持多标签页工作流，适合数据库调试、脚本执行和部署操作。</p>
                </article>
                <article class="capability-card">
                    <h3>智能辅助</h3>
                    <p>支持生成 SQL、解释查询和辅助数据分析，作为数据库工作的提效能力使用。</p>
                </article>
            </div>
        </section>

        <section class="section-block value-section">
            <div class="section-heading">
                <p class="section-label">为什么选择 OnetCli</p>
                <h2>更少切换，更快进入真正的工作状态</h2>
            </div>
            <div class="value-list">
                <article>
                    <h3>统一工作流</h3>
                    <p>数据库、远程连接和终端不再分散在多个工具里，常用操作可以在同一个桌面工作台里连续完成。</p>
                </article>
                <article>
                    <h3>桌面体验</h3>
                    <p>作为跨平台桌面应用，OnetCli 更适合重度日常使用，也更贴近工程师的实际工作节奏。</p>
                </article>
                <article>
                    <h3>面向真实任务</h3>
                    <p>AI 在这里围绕 SQL 生成、查询理解和数据分析提供辅助，服务真实高频任务，而不是喧宾夺主地占据产品主叙事。</p>
                </article>
            </div>
        </section>

        <section class="section-block download-section">
            <div class="section-heading">
                <p class="section-label">下载</p>
                <h2>先从 GitHub Releases 获取最新版本</h2>
                <p>
                    当前下载入口统一使用 GitHub Releases，便于快速上线和版本管理。
                </p>
            </div>
            <div class="download-panel">
                <div>
                    <h3>支持平台</h3>
                    <p>macOS、Windows、Linux</p>
                </div>
                <a class="primary-action" href="https://github.com/feigeCode/onetcli/releases" target="_blank" rel="noreferrer">
                    <span>前往 GitHub Releases</span>
                </a>
            </div>
        </section>

        <section class="section-block faq-section">
            <div class="section-heading">
                <p class="section-label">FAQ</p>
                <h2>关于 OnetCli 的几个高频问题</h2>
            </div>
            <div class="faq-list">
                <article>
                    <h3>OnetCli 是什么？</h3>
                    <p>OnetCli 是一个跨平台桌面客户端，用来统一处理数据库、SSH / SFTP 与终端操作，并提供面向数据库场景的智能辅助能力。</p>
                </article>
                <article>
                    <h3>OnetCli 支持哪些数据库？</h3>
                    <p>当前仓库文档中明确提到 PostgreSQL、MySQL、SQLite、SQL Server、Oracle、ClickHouse、Redis 和 MongoDB。</p>
                </article>
                <article>
                    <h3>OnetCli 是否支持 SSH 和 SFTP？</h3>
                    <p>支持。OnetCli 内置 SSH 终端与 SFTP 文件管理器，适合远程排查与文件操作。</p>
                </article>
                <article>
                    <h3>OnetCli 是否支持 AI 生成 SQL？</h3>
                    <p>支持。智能辅助能力可用于自然语言生成 SQL、解释查询以及数据分析提效，核心目标是让数据库工作更快、更顺。</p>
                </article>
            </div>
        </section>

        <section class="section-block updates-section">
            <div class="section-heading">
                <p class="section-label">最近更新</p>
                <h2>保持持续迭代，而不是一次性发布</h2>
            </div>
            <div class="update-list">
                <article>
                    <h3>数据库体验持续增强</h3>
                    <p>围绕表设计器、数据库树和数据操作流持续改进，降低日常管理成本。</p>
                </article>
                <article>
                    <h3>终端与远程连接能力迭代</h3>
                    <p>在终端、SSH、SFTP 等模块持续补齐高频交互细节。</p>
                </article>
                <article>
                    <h3>查看完整版本记录</h3>
                    <p><a :href="withBase('/changelog')">进入更新日志</a></p>
                </article>
            </div>
        </section>

        <script type="application/ld+json">
            {{ structuredData }}
        </script>
    </main>
</template>

<script setup>
import { withBase } from "vitepress";

const heroImage = withBase("/screenshots/app.png");
const databaseImage = withBase("/screenshots/database.png");
const sshImage = withBase("/screenshots/ssh.png");
const chatdbImage = withBase("/screenshots/chatdb.png");
const structuredData = JSON.stringify(
    [
        {
            "@context": "https://schema.org",
            "@type": "SoftwareApplication",
            name: "OnetCli",
            applicationCategory: "DeveloperApplication",
            operatingSystem: "macOS, Windows, Linux",
            description:
                "OnetCli 是一个统一管理数据库、SSH / SFTP 与终端操作，并提供 SQL 生成与数据分析辅助能力的跨平台桌面客户端。",
            downloadUrl: "https://github.com/feigeCode/onetcli/releases",
        },
        {
            "@context": "https://schema.org",
            "@type": "FAQPage",
            mainEntity: [
                {
                    "@type": "Question",
                    name: "OnetCli 是什么？",
                    acceptedAnswer: {
                        "@type": "Answer",
                        text: "OnetCli 是一个跨平台桌面客户端，用来统一处理数据库、SSH / SFTP 与终端操作，并提供面向数据库场景的智能辅助能力。",
                    },
                },
                {
                    "@type": "Question",
                    name: "OnetCli 是否支持 SSH 和 SFTP？",
                    acceptedAnswer: {
                        "@type": "Answer",
                        text: "支持。OnetCli 内置 SSH 终端与 SFTP 文件管理器，适合远程排查与文件操作。",
                    },
                },
                {
                    "@type": "Question",
                    name: "OnetCli 是否支持 AI 生成 SQL？",
                    acceptedAnswer: {
                        "@type": "Answer",
                        text: "支持。智能辅助能力可用于自然语言生成 SQL、解释查询以及辅助数据分析，核心目标是让数据库工作更快、更顺。",
                    },
                },
            ],
        },
    ],
    null,
    2,
);
</script>

<style lang="scss">
@reference "./.vitepress/theme/style.css";

.home-page {
    @apply flex flex-col gap-8 pb-16;
    background:
        radial-gradient(circle at top left, rgba(39, 130, 103, 0.12), transparent 28%),
        radial-gradient(circle at top right, rgba(242, 184, 72, 0.12), transparent 24%);
}

.hero,
.section-block {
    @apply border border-(--border) rounded-[28px] overflow-hidden;
}

.hero {
    @apply grid gap-8 px-6 py-8 shadow-[0_20px_60px_rgba(17,24,39,0.08)] lg:grid-cols-[1.02fr_0.98fr] lg:px-10 lg:py-12;
    background:
        radial-gradient(circle at top left, rgba(39, 130, 103, 0.22), transparent 30%),
        radial-gradient(circle at right center, rgba(242, 184, 72, 0.22), transparent 24%),
        linear-gradient(135deg, #fdfcf8 0%, #f7f5ee 52%, #f4efe4 100%);
    color: #18212f;

    .hero-copy {
        @apply flex flex-col justify-center gap-5;
    }

    .eyebrow,
    .section-label {
        @apply text-xs tracking-[0.28em] uppercase text-[#4d6b60];
    }

    h1 {
        @apply m-0 max-w-3xl text-4xl leading-tight font-semibold lg:text-[3.8rem];
        font-family: "Space Grotesk", "PingFang SC", "Microsoft YaHei", sans-serif;
        letter-spacing: -0.04em;
    }

    .hero-description {
        @apply m-0 max-w-2xl text-base leading-7 text-[#52606d] lg:text-lg;
    }

    .hero-note {
        @apply flex flex-wrap gap-2;

        span {
            @apply inline-flex items-center rounded-full border border-[#d5ddd4] bg-white/70 px-3 py-1 text-sm font-medium text-[#29473e];
        }
    }

    .hero-actions {
        @apply flex flex-wrap gap-3;
    }

    .hero-metrics {
        @apply grid gap-4 pt-3 md:grid-cols-2;

        div {
            @apply rounded-2xl border border-[#d9e1d7] bg-white/72 px-4 py-4 backdrop-blur;
        }

        dt {
            @apply text-sm text-[#6a737d];
        }

        dd {
            @apply mt-2 text-base font-medium text-[#18212f];
        }
    }

    .hero-visual {
        @apply flex items-center justify-center;

        img {
            @apply w-full rounded-[24px] border border-white/80 bg-white p-2 shadow-[0_28px_80px_rgba(17,24,39,0.12)];
        }
    }
}

.primary-action,
.secondary-action {
    @apply inline-flex items-center justify-center rounded-full px-5 py-3 text-sm font-medium no-underline transition-all duration-200;
}

.primary-action span,
.secondary-action span {
    position: relative;
    z-index: 1;
}

.primary-action {
    @apply bg-[#18212f] shadow-[0_16px_32px_rgba(24,33,47,0.18)] hover:-translate-y-0.5 hover:bg-[#0f1722];
    color: #ffffff;

    &:visited,
    &:hover,
    &:active {
        color: #ffffff;
    }

    span {
        color: #ffffff !important;
        text-shadow: 0 1px 1px rgba(0, 0, 0, 0.16);
    }
}

.secondary-action {
    @apply border border-[#c9d4cb] bg-white/78 text-[#1d4d41] hover:-translate-y-0.5 hover:border-[#9fbaa9] hover:bg-white;

    span {
        color: #1d4d41;
    }
}

.section-block {
    @apply bg-(--background) px-6 py-8 shadow-[0_18px_50px_rgba(17,24,39,0.05)] lg:px-10;
}

.section-heading {
    @apply mb-6 flex flex-col gap-3;

    .section-label {
        @apply text-xs tracking-[0.28em] uppercase text-(--muted-foreground);
    }

    h2 {
        @apply m-0 text-3xl font-semibold text-(--foreground);
        font-family: "Space Grotesk", "PingFang SC", "Microsoft YaHei", sans-serif;
    }

    p {
        @apply m-0 max-w-3xl text-base leading-7 text-(--muted-foreground);
    }
}

.screenshot-grid,
.capability-grid,
.value-list,
.faq-list,
.update-list {
    @apply grid gap-4;
}

.screenshot-grid {
    @apply lg:grid-cols-3;
}

.capability-grid {
    @apply md:grid-cols-2;
}

.value-list {
    @apply lg:grid-cols-3;
}

.faq-list,
.update-list {
    @apply md:grid-cols-2;
}

.screenshot-card,
.capability-card,
.value-list article,
.faq-list article,
.update-list article,
.download-panel {
    @apply rounded-[24px] border border-(--border) bg-(--secondary) p-5 shadow-[0_12px_30px_rgba(17,24,39,0.04)];
}

.screenshot-card {
    @apply flex flex-col gap-4;

    img {
        @apply aspect-[16/10] w-full rounded-[18px] object-cover;
    }
}

.download-panel {
    @apply flex flex-col gap-4 md:flex-row md:items-center md:justify-between;
}

.section-block h3 {
    @apply m-0 text-xl font-semibold text-(--foreground);
}

.section-block p,
.section-block li {
    @apply text-(--muted-foreground);
}

@media (max-width: 767px) {
    .hero {
        @apply rounded-[24px];
    }

    .section-block {
        @apply rounded-[24px];
    }
}
</style>
