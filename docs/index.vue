<template>
    <main class="home-page">
        <section class="hero">
            <div class="hero-copy">
                <p class="eyebrow">Pure Rust · GPUI · GPU Rendered · No WebView</p>
                <h1>纯 Rust 构建的高性能一体化运维工作台</h1>
                <p class="hero-description">
                    OnetCli 基于 GPUI 与 Rust 原生技术栈开发，走 GPU 加速渲染路线，不依赖 WebView。
                    数据库、Redis、MongoDB、SSH/SFTP、终端会话、远程桌面（RDP/VNC）和 AI 助手，在一个高性能桌面工作台里连续完成。
                </p>
                <div class="hero-actions">
                    <a class="primary-action" href="https://github.com/feigeCode/onetcli/releases" target="_blank" rel="noreferrer">
                        <span>下载 OnetCli</span>
                    </a>
                    <a class="secondary-action" :href="withBase('/guide')">
                        <span>查看文档</span>
                    </a>
                </div>
                <div class="hero-chips">
                    <span>多数据库连接</span>
                    <span>国产数据库扩展</span>
                    <span>Redis / MongoDB</span>
                    <span>SSH / SFTP</span>
                    <span>远程桌面 RDP / VNC</span>
                    <span>本地与远程终端</span>
                    <span>AI SQL 助手</span>
                    <span>纯 Rust 原生渲染</span>
                </div>
            </div>

            <div class="product-shell">
                <img :src="heroImage" alt="OnetCli 主应用工作台总览" />
            </div>
        </section>

        <section class="metrics" aria-label="OnetCli product highlights">
            <article>
                <strong>1 个</strong>
                <span>统一工作台覆盖数据库、远程连接、文件传输和终端</span>
            </article>
            <article>
                <strong>Rust</strong>
                <span>GPUI 原生桌面界面，GPU 加速渲染，无 WebView 外壳</span>
            </article>
            <article>
                <strong>Redis</strong>
                <span>Key 浏览、值查看、集群场景和日常缓存排查</span>
            </article>
            <article>
                <strong>RDP / VNC</strong>
                <span>应用内打开 RDP 与 VNC 远程桌面，远程运维不再跳出工作台</span>
            </article>
        </section>

        <section class="section-block">
            <div class="section-heading">
                <h2>用户第一眼就能看到真实产品，而不是抽象概念</h2>
                <p>页面大部分面积用于展示实际界面。每张图都对应一个明确场景，用户扫一眼就知道 OnetCli 能解决什么问题。</p>
            </div>
            <div class="image-grid">
                <article class="shot large">
                    <img :src="databaseImage" alt="OnetCli 数据库对象浏览与 SQL 助手" />
                    <div class="caption">
                        <h3>数据库对象浏览 + SQL 助手</h3>
                        <p>
                            左侧浏览库表结构，中间查看对象和数据，右侧由 AI 辅助理解字段、生成 SQL、解释操作建议，
                            例如根据业务需求生成 <code>ALTER TABLE login_user ADD COLUMN email VARCHAR(255)</code> 这类变更语句。
                        </p>
                    </div>
                </article>
                <article class="shot">
                    <img :src="sshImage" alt="OnetCli SSH 远程终端与 AI 助手" />
                    <div class="caption">
                        <h3>远程终端排查</h3>
                        <p>SSH 会话与 AI 助手并排，适合查看磁盘、日志、进程、服务状态等日常运维动作。</p>
                    </div>
                </article>
                <article class="shot">
                    <img :src="chatdbImage" alt="OnetCli 查询结果分析界面" />
                    <div class="caption">
                        <h3>查询结果分析</h3>
                        <p>让 AI 围绕查询结果输出结论、图表和下一步建议，把数据分析放回实际数据库工作流。</p>
                    </div>
                </article>
            </div>
        </section>

        <section class="section-block">
            <div class="section-heading">
                <h2>把高频工具合成一条原生工作路径</h2>
                <p>OnetCli 的重点不是用网页壳堆功能，而是用 Rust 与 GPUI 做真正的桌面体验，让常见任务从连接到执行都保持上下文和性能。</p>
            </div>
            <div class="feature-grid">
                <article class="feature-card"><span>DB</span><h3>数据库连接管理</h3><p>按项目或环境组织连接，快速进入 MySQL、PostgreSQL、SQLite、SQL Server、Oracle、ClickHouse、DuckDB 等数据库工作区。</p></article>
                <article class="feature-card"><span>+</span><h3>国产与扩展数据库</h3><p>通过扩展市场按需安装达梦 DM、金仓 KingbaseES、GBase 8s、OceanBase、openGauss、Apache IoTDB，以及免 Instant Client 的纯 Go Oracle 驱动。</p></article>
                <article class="feature-card"><span>RD</span><h3>远程桌面 RDP / VNC</h3><p>应用内打开 RDP 与 VNC 会话，连接 Windows 桌面或任意 VNC 服务端，远程操作与数据库、终端、文件共用同一上下文。</p></article>
                <article class="feature-card"><span>SQL</span><h3>SQL 编辑与对象浏览</h3><p>浏览表、视图、函数和字段信息，执行 SQL，查看结果，并在同一界面理解结构和数据。</p></article>
                <article class="feature-card"><span>RS</span><h3>纯 Rust 原生桌面</h3><p>基于 GPUI 构建，采用 GPU 渲染，不依赖 WebView，目标是更低资源占用、更快响应和更稳定的桌面体验。</p></article>
                <article class="feature-card"><span>KV</span><h3>Redis 工作台</h3><p>提供 Redis Key 浏览、值查看、缓存排查和集群场景支持，让缓存数据也纳入统一工作流。</p></article>
                <article class="feature-card"><span>MO</span><h3>MongoDB Explorer</h3><p>支持集合浏览、文档查看和查询操作，关系型数据库与 NoSQL 数据源可以在同一个客户端里处理。</p></article>
                <article class="feature-card"><span>SSH</span><h3>SSH 远程终端</h3><p>连接服务器后直接进入远程 Shell，适合线上排查、部署确认、日志查看和临时命令执行。</p></article>
                <article class="feature-card"><span>SFTP</span><h3>SFTP 文件传输</h3><p>远程文件操作与 SSH 连接复用同一上下文，传输配置、脚本、日志和备份文件更自然。</p></article>
                <article class="feature-card"><span>AI</span><h3>AI 数据库助手</h3><p>用自然语言生成 SQL、解释字段含义、分析查询结果，把智能能力放在数据任务旁边。</p></article>
                <article class="feature-card"><span>CLI</span><h3>本地终端工作流</h3><p>终端、数据库和远程会话都在桌面应用内管理，更适合高频、多窗口、长时间使用。</p></article>
            </div>
        </section>

        <section class="section-block">
            <div class="spotlight">
                <div class="spotlight-copy">
                    <h2>不是网页套壳，是原生 Rust 应用</h2>
                    <p>
                        OnetCli 使用 GPUI 构建界面，沿用 Zed 生态的高性能原生 UI 路线。
                        桌面端通过 GPU 加速渲染，而不是把一个 Web 应用塞进 WebView。
                    </p>
                    <div class="check-list">
                        <div><b>✓</b><span>Rust 2024 技术栈，界面、终端、数据库连接和远程能力都更贴近原生桌面。</span></div>
                        <div><b>✓</b><span>macOS 使用 Metal 渲染，Linux 使用 Vulkan 渲染，强调高性能与低延迟交互。</span></div>
                        <div><b>✓</b><span>适合长时间打开、频繁切换连接和处理大量工程上下文的重度用户。</span></div>
                    </div>
                </div>
                <article class="shot">
                    <img :src="monitorImage" alt="OnetCli 原生渲染监控图表" />
                    <div class="caption">
                        <h3>原生渲染图表与监控</h3>
                        <p>内置简易服务器监控和原生渲染图表，用桌面应用的方式呈现运行状态和关键指标。</p>
                    </div>
                </article>
            </div>
        </section>

        <section class="section-block">
            <div class="section-heading">
                <h2>Redis 和 MongoDB 不是补充功能，而是完整入口</h2>
                <p>很多用户会先看自己常用的数据源是否被支持。这里直接展示 Redis 与 MongoDB，避免只在文字里一笔带过。</p>
            </div>
            <div class="split-gallery">
                <article class="shot">
                    <img :src="redisImage" alt="OnetCli Redis 管理界面" />
                    <div class="caption">
                        <h3>Redis Key 浏览与值查看</h3>
                        <p>用于缓存排查、Key 检索、值检查和集群场景管理，和数据库、SSH 工作流共用同一个桌面入口。</p>
                    </div>
                </article>
                <article class="shot">
                    <img :src="mongodbImage" alt="OnetCli MongoDB 管理界面" />
                    <div class="caption">
                        <h3>MongoDB 集合与文档浏览</h3>
                        <p>浏览集合、查看文档、执行查询，让 NoSQL 数据源和关系型数据库一起纳入 OnetCli 的统一工作台。</p>
                    </div>
                </article>
            </div>
        </section>

        <section class="section-block">
            <div class="section-heading">
                <h2>更多真实界面，覆盖完整运维链路</h2>
                <p>继续展示 SFTP、远程编辑、ER 图和服务器监控，减少纯文字介绍带来的疲劳。</p>
            </div>
            <div class="split-gallery">
                <article class="shot">
                    <img :src="sftpImage" alt="OnetCli SFTP 文件管理界面" />
                    <div class="caption">
                        <h3>SFTP 文件管理</h3>
                        <p>远程目录浏览、文件传输和服务器排查可以放在同一套连接上下文里完成。</p>
                    </div>
                </article>
                <article class="shot">
                    <img :src="sftpSidebarImage" alt="OnetCli 终端内置 SFTP 侧栏" />
                    <div class="caption">
                        <h3>终端内置 SFTP 侧栏</h3>
                        <p>终端操作时可以直接打开 SFTP 侧栏，并支持文件拖拽上传，排查和传输不再割裂。</p>
                    </div>
                </article>
            </div>
        </section>

        <section class="section-block">
            <div class="image-grid">
                <article class="shot large">
                    <img :src="remoteEditorImage" alt="OnetCli 远程文件编辑器" />
                    <div class="caption">
                        <h3>远程文件直接编辑</h3>
                        <p>从应用内打开远程文件，支持语法高亮和自动补全。对配置文件、脚本和日志片段的处理更接近完整 IDE 体验。</p>
                    </div>
                </article>
                <article class="shot">
                    <img :src="erImage" alt="OnetCli ER Diagram" />
                    <div class="caption">
                        <h3>ER Diagram</h3>
                        <p>通过 ER 图理解表关系，适合接手新系统、梳理业务数据结构和沟通数据库设计。</p>
                    </div>
                </article>
                <article class="shot">
                    <img :src="monitorImage" alt="OnetCli 服务器监控界面" />
                    <div class="caption">
                        <h3>服务器监控</h3>
                        <p>查看基础服务器状态和趋势图，把远程连接后的常见检查动作产品化。</p>
                    </div>
                </article>
            </div>
        </section>

        <section class="section-block">
            <div class="section-heading">
                <h2>典型工作流更短，用户更容易对号入座</h2>
                <p>围绕“连接、定位、执行、解释”四步来讲，让产品和日常运维、开发、数据工作自然对应起来。</p>
            </div>
            <div class="workflow">
                <article><b>1</b><h3>打开工作区</h3><p>从保存的连接进入数据库、服务器或本地终端，快速回到上次工作的环境。</p></article>
                <article><b>2</b><h3>定位对象</h3><p>查看库表、字段、远程目录或终端输出，把问题定位到具体数据和系统上下文。</p></article>
                <article><b>3</b><h3>执行操作</h3><p>运行 SQL、执行命令、传输文件或查看日志，核心操作在同一桌面应用内完成。</p></article>
                <article><b>4</b><h3>AI 辅助</h3><p>生成查询、解释结果、建议命令或整理排查思路，让下一步动作更明确。</p></article>
            </div>
        </section>

        <section class="section-block">
            <div class="section-heading">
                <h2>面向这些高频用户场景</h2>
                <p>比起只说“功能强大”，直接告诉用户可以用它完成哪些真实工作，更容易形成下载意愿。</p>
            </div>
            <div class="use-grid">
                <article><h3>后端开发</h3><p>本地开发时同时查看数据库、执行迁移脚本、连接测试服务器和分析接口数据。</p></article>
                <article><h3>运维排查</h3><p>SSH 登录服务器、查看磁盘和日志、传输配置文件，并把命令说明交给 AI 辅助整理。</p></article>
                <article><h3>缓存排查</h3><p>进入 Redis 查看 Key、值和集群状态，快速确认缓存数据是否符合预期。</p></article>
                <article><h3>NoSQL 探索</h3><p>在 MongoDB 中浏览集合和文档，结合查询能力快速定位业务数据。</p></article>
                <article><h3>数据库管理</h3><p>管理多种数据库连接，查看对象结构、执行查询、维护表字段和日常数据检查。</p></article>
                <article><h3>项目交付</h3><p>跨环境切换时减少工具切换，把连接、命令、文件和数据操作统一到一个桌面入口。</p></article>
            </div>
        </section>

        <section class="section-block faq-section">
            <div class="section-heading">
                <h2>FAQ</h2>
                <p>在首屏和截图之后继续消除疑虑：支持什么、怎么开始、AI 在哪里发挥作用。</p>
            </div>
            <div class="faq-grid">
                <article><h3>OnetCli 是数据库客户端吗？</h3><p>它包含数据库客户端能力，但定位更大：数据库、Redis、MongoDB、SSH/SFTP、终端和 AI 工作流统一在一个桌面工作台里。</p></article>
                <article><h3>为什么强调 Rust 和无 WebView？</h3><p>这说明 OnetCli 不是网页套壳。它基于 GPUI 构建，走 GPU 加速的原生桌面渲染路线，更适合追求性能和长期稳定使用的用户。</p></article>
                <article><h3>AI 会不会喧宾夺主？</h3><p>不会。AI 作为辅助层贴近 SQL、查询结果和终端排查场景，核心操作仍然围绕真实工程任务。</p></article>
            </div>
        </section>

        <section class="section-block download-section">
            <div class="download-panel">
                <div>
                    <h2>先下载桌面版，5 分钟跑通第一个连接</h2>
                    <p>
                        体验纯 Rust 原生性能，将数据库、Redis、MongoDB、SSH/SFTP、远程桌面、远程编辑和 AI 工作流统一到一个桌面入口。
                    </p>
                </div>
                <div class="download-actions">
                    <a class="primary-action" href="https://github.com/feigeCode/onetcli/releases" target="_blank" rel="noreferrer">
                        <span>前往 GitHub Releases</span>
                    </a>
                    <a class="secondary-action" :href="withBase('/guide')">
                        <span>阅读快速开始</span>
                    </a>
                </div>
            </div>
        </section>

    </main>
</template>

<script setup>
import { withBase } from "vitepress";
import { onMounted, onUnmounted } from "vue";

const heroImage = withBase("/screenshots/app.png");
const databaseImage = withBase("/screenshots/database.png");
const sshImage = withBase("/screenshots/ssh.png");
const chatdbImage = withBase("/screenshots/chatdb.png");
const redisImage = withBase("/screenshots/redis.png");
const mongodbImage = withBase("/screenshots/mongodb.png");
const sftpImage = withBase("/screenshots/sftp.png");
const sftpSidebarImage = withBase("/screenshots/sftp_sidebar.png");
const remoteEditorImage = withBase("/screenshots/remote_file_editor.png");
const erImage = withBase("/screenshots/er.png");
const monitorImage = withBase("/screenshots/monitor.png");

const structuredData = JSON.stringify(
    [
        {
            "@context": "https://schema.org",
            "@type": "SoftwareApplication",
            name: "OnetCli",
            applicationCategory: "DeveloperApplication",
            operatingSystem: "macOS, Windows, Linux",
            description:
                "OnetCli 是一个纯 Rust 构建、基于 GPUI 和 GPU 渲染的一体化运维工作台，用于统一管理数据库、Redis、MongoDB、SSH/SFTP、终端与 AI 工作流。",
            downloadUrl: "https://github.com/feigeCode/onetcli/releases",
            featureList: [
                "Pure Rust GPUI desktop application",
                "GPU rendered native UI without WebView",
                "Database management",
                "Redis key browsing",
                "MongoDB document browsing",
                "Domestic and specialty database extensions",
                "SSH and SFTP",
                "RDP and VNC remote desktop",
                "AI SQL assistant",
            ],
        },
        {
            "@context": "https://schema.org",
            "@type": "FAQPage",
            mainEntity: [
                {
                    "@type": "Question",
                    name: "OnetCli 是数据库客户端吗？",
                    acceptedAnswer: {
                        "@type": "Answer",
                        text: "OnetCli 包含数据库客户端能力，但定位更大：数据库、Redis、MongoDB、SSH/SFTP、终端和 AI 工作流统一在一个桌面工作台里。",
                    },
                },
                {
                    "@type": "Question",
                    name: "为什么强调 Rust 和无 WebView？",
                    acceptedAnswer: {
                        "@type": "Answer",
                        text: "OnetCli 基于 GPUI 构建，走 GPU 加速的原生桌面渲染路线，不依赖 WebView，更适合追求性能和长期稳定使用的用户。",
                    },
                },
                {
                    "@type": "Question",
                    name: "OnetCli 是否支持 Redis 和 MongoDB？",
                    acceptedAnswer: {
                        "@type": "Answer",
                        text: "支持。OnetCli 提供 Redis Key 浏览、值查看、集群场景支持，也提供 MongoDB 集合浏览、文档查看和查询能力。",
                    },
                },
                {
                    "@type": "Question",
                    name: "是否支持国产数据库和远程桌面？",
                    acceptedAnswer: {
                        "@type": "Answer",
                        text: "支持。OnetCli 通过扩展市场提供达梦 DM、金仓 KingbaseES、GBase 8s、OceanBase、openGauss、Apache IoTDB 等国产与特色数据库驱动，并可打开 RDP 和 VNC 远程桌面会话，远程操作与数据库、终端工作流共用同一个桌面工作台。",
                    },
                },
            ],
        },
    ],
    null,
    2,
);

let structuredDataElement;

onMounted(() => {
    structuredDataElement = document.createElement("script");
    structuredDataElement.type = "application/ld+json";
    structuredDataElement.textContent = structuredData;
    document.head.appendChild(structuredDataElement);
});

onUnmounted(() => {
    structuredDataElement?.remove();
});
</script>

<style lang="scss">
@reference "./.vitepress/theme/style.css";

.home-page {
    position: relative;
    min-height: 100vh;
    margin: -48px calc(50% - 50vw) 0;
    padding: 72px max(24px, calc((100vw - 1200px) / 2)) 96px;
    overflow: hidden;
    color: #f8fafc;
    background:
        radial-gradient(circle at 50% -8%, rgba(224, 90, 71, 0.14), transparent 34rem),
        radial-gradient(circle at 76% 10%, rgba(99, 102, 241, 0.12), transparent 32rem),
        radial-gradient(circle at 16% 36%, rgba(224, 90, 71, 0.055), transparent 30rem),
        linear-gradient(180deg, #0b0d13 0%, #0b0d13 42%, #07090f 100%);

    &::before {
        content: "";
        position: absolute;
        left: 50%;
        top: -120px;
        width: 720px;
        height: 360px;
        transform: translateX(-50%);
        background: radial-gradient(circle, rgba(224, 90, 71, 0.11) 0%, rgba(99, 102, 241, 0.07) 48%, transparent 100%);
        filter: blur(78px);
        pointer-events: none;
    }
}

.hero,
.metrics,
.section-block {
    position: relative;
    z-index: 1;
}

.hero {
    display: grid;
    gap: 42px;
    text-align: center;
}

.hero-copy {
    max-width: 980px;
    margin: 0 auto;
}

.eyebrow {
    display: inline-flex;
    align-items: center;
    min-height: 34px;
    border: 1px solid rgba(224, 90, 71, 0.28);
    border-radius: 999px;
    padding: 0 14px;
    background: rgba(19, 22, 34, 0.82);
    color: #f08b79;
    font-size: 13px;
    font-weight: 680;
}

h1 {
    max-width: 980px;
    margin: 24px auto 0;
    font-size: clamp(42px, 7vw, 84px);
    line-height: 0.98;
    font-weight: 880;
    letter-spacing: -0.025em;
    color: #ffffff;
    text-shadow: 0 20px 80px rgba(0, 0, 0, 0.45);
}

.hero-description {
    max-width: 800px;
    margin: 24px auto 0;
    color: #a8b0c0;
    font-size: clamp(16px, 2vw, 20px);
    line-height: 1.72;
}

.hero-actions,
.download-actions {
    display: flex;
    justify-content: center;
    gap: 14px;
    margin-top: 34px;
    flex-wrap: wrap;
}

.primary-action,
.secondary-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: 48px;
    border-radius: 12px;
    padding: 0 22px;
    font-size: 14px;
    font-weight: 740;
    text-decoration: none !important;
    transition: transform 0.2s ease, border-color 0.2s ease, background 0.2s ease;

    &:hover {
        transform: translateY(-1px);
    }
}

.primary-action {
    background: #ffffff;
    color: #0b0d13 !important;
    box-shadow: 0 18px 46px rgba(255, 255, 255, 0.08), 0 0 42px rgba(224, 90, 71, 0.12);
}

.secondary-action {
    border: 1px solid rgba(34, 38, 56, 0.92);
    background: rgba(19, 22, 34, 0.86);
    color: #d5d9e4 !important;
}

.hero-chips {
    display: flex;
    justify-content: center;
    gap: 12px;
    flex-wrap: wrap;
    margin-top: 24px;

    span {
        border: 1px solid rgba(34, 38, 56, 0.9);
        border-radius: 999px;
        padding: 8px 12px;
        background: rgba(19, 22, 34, 0.72);
        color: #aeb6c6;
        font-size: 13px;
    }
}

.product-shell {
    border: 1px solid rgba(34, 38, 56, 0.92);
    border-radius: 22px;
    overflow: hidden;
    background: linear-gradient(180deg, #131622, #0d1018);
    box-shadow: 0 42px 120px rgba(0, 0, 0, 0.68), 0 0 90px rgba(224, 90, 71, 0.08);
}

.product-shell img {
    display: block;
    width: 100%;
    aspect-ratio: 16 / 9;
    object-fit: cover;
    object-position: left top;
}

.metrics {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
    margin: 34px 0 84px;

    article {
        min-height: 118px;
        border: 1px solid rgba(34, 38, 56, 0.92);
        border-radius: 16px;
        padding: 20px;
        background: rgba(19, 22, 34, 0.82);
        backdrop-filter: blur(16px);
    }

    strong {
        display: block;
        margin-bottom: 8px;
        color: #ffffff;
        font-size: 28px;
    }

    span {
        color: #8f98aa;
        font-size: 13px;
        line-height: 1.5;
    }
}

.section-block {
    padding-bottom: 92px;
}

.section-heading {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 32px;
    margin-bottom: 24px;

    h2 {
        max-width: 720px;
        margin: 0;
        color: #ffffff;
        font-size: clamp(30px, 4vw, 48px);
        line-height: 1.08;
        font-weight: 820;
    }

    p {
        max-width: 430px;
        margin: 0;
        color: #8f98aa;
        line-height: 1.7;
    }
}

.image-grid {
    display: grid;
    grid-template-columns: 1.12fr 0.88fr;
    gap: 16px;
}

.split-gallery {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 16px;
}

.shot,
.feature-card,
.workflow article,
.use-grid article,
.faq-grid article,
.download-panel {
    border: 1px solid rgba(34, 38, 56, 0.92);
    background: rgba(19, 22, 34, 0.86);
    box-shadow: 0 26px 90px rgba(0, 0, 0, 0.38);
}

.shot,
.feature-card,
.workflow article,
.use-grid article,
.faq-grid article {
    border-radius: 18px;
    transition: transform 0.22s ease, border-color 0.22s ease, box-shadow 0.22s ease;

    &:hover {
        transform: translateY(-2px);
        border-color: rgba(224, 90, 71, 0.34);
        box-shadow: 0 30px 90px rgba(0, 0, 0, 0.45), 0 0 52px rgba(224, 90, 71, 0.055);
    }
}

.shot {
    overflow: hidden;

    &.large {
        grid-row: span 2;
    }

    img {
        display: block;
        width: 100%;
        aspect-ratio: 16 / 10;
        object-fit: cover;
        object-position: left top;
    }

    &.large img {
        aspect-ratio: 16 / 12.3;
    }
}

.caption {
    padding: 18px;
    border-top: 1px solid rgba(34, 38, 56, 0.78);
    background: linear-gradient(180deg, rgba(19, 22, 34, 0.98), rgba(15, 18, 28, 0.98));

    h3 {
        margin: 0 0 8px;
        color: #ffffff;
        font-size: 18px;
    }

    p {
        margin: 0;
        color: #8f98aa;
        font-size: 14px;
        line-height: 1.66;
    }

    code {
        color: #f08b79;
        font-size: 13px;
    }
}

.feature-grid,
.use-grid,
.faq-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 14px;
}

.feature-card,
.use-grid article,
.faq-grid article {
    padding: 22px;

    h3 {
        margin: 0 0 12px;
        color: #ffffff;
        font-size: 18px;
    }

    p {
        margin: 0;
        color: #8f98aa;
        font-size: 14px;
        line-height: 1.7;
    }
}

.feature-card {
    min-height: 214px;

    span {
        width: 42px;
        height: 42px;
        border: 1px solid rgba(224, 90, 71, 0.16);
        border-radius: 13px;
        display: grid;
        place-items: center;
        margin-bottom: 18px;
        background: rgba(224, 90, 71, 0.1);
        color: #f08b79;
        font-size: 13px;
        font-weight: 840;
    }
}

.spotlight {
    display: grid;
    grid-template-columns: 0.9fr 1.1fr;
    gap: 18px;
    align-items: stretch;
}

.spotlight-copy {
    border: 1px solid rgba(34, 38, 56, 0.92);
    border-radius: 20px;
    padding: 28px;
    background:
        radial-gradient(circle at 0% 0%, rgba(224, 90, 71, 0.12), transparent 18rem),
        radial-gradient(circle at 100% 100%, rgba(99, 102, 241, 0.09), transparent 16rem),
        rgba(19, 22, 34, 0.86);
    box-shadow: 0 26px 90px rgba(0, 0, 0, 0.36);

    h2 {
        margin: 0 0 16px;
        color: #ffffff;
        font-size: clamp(30px, 4vw, 48px);
        line-height: 1.08;
    }

    p {
        margin: 0 0 18px;
        color: #c6ccda;
        line-height: 1.72;
    }
}

.check-list {
    display: grid;
    gap: 12px;
    margin-top: 22px;

    div {
        display: flex;
        gap: 10px;
        align-items: flex-start;
        color: #cbd5e1;
        font-size: 14px;
        line-height: 1.55;
    }

    b {
        flex: 0 0 auto;
        width: 22px;
        height: 22px;
        border-radius: 999px;
        display: grid;
        place-items: center;
        background: rgba(224, 90, 71, 0.14);
        color: #f08b79;
        font-size: 12px;
    }
}

.workflow {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 14px;

    article {
        min-height: 190px;
        padding: 20px;
    }

    b {
        width: 34px;
        height: 34px;
        border: 1px solid rgba(99, 102, 241, 0.18);
        border-radius: 11px;
        display: grid;
        place-items: center;
        margin-bottom: 18px;
        background: rgba(99, 102, 241, 0.12);
        color: #b8b9ff;
    }

    h3 {
        margin: 0 0 10px;
        color: #ffffff;
        font-size: 17px;
    }

    p {
        margin: 0;
        color: #8f98aa;
        font-size: 14px;
        line-height: 1.65;
    }
}

.use-grid article {
    min-height: 170px;
}

.download-panel {
    border-radius: 22px;
    padding: 34px;
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    gap: 28px;
    background:
        radial-gradient(circle at 10% 0%, rgba(224, 90, 71, 0.14), transparent 24rem),
        radial-gradient(circle at 86% 40%, rgba(99, 102, 241, 0.11), transparent 22rem),
        linear-gradient(180deg, rgba(19, 22, 34, 0.98), rgba(22, 26, 41, 0.98));

    h2 {
        margin: 0 0 12px;
        color: #ffffff;
        font-size: clamp(28px, 4vw, 42px);
        line-height: 1.1;
    }

    p {
        max-width: 720px;
        margin: 0;
        color: #8f98aa;
        line-height: 1.7;
    }
}

.download-actions {
    margin-top: 0;
}

@media (max-width: 920px) {
    .home-page {
        margin-top: -24px;
        padding-top: 48px;
    }

    .metrics,
    .image-grid,
    .feature-grid,
    .spotlight,
    .workflow,
    .split-gallery,
    .use-grid,
    .faq-grid,
    .download-panel {
        grid-template-columns: 1fr;
    }

    .metrics {
        margin-top: 24px;
    }

    .section-heading {
        display: block;

        p {
            margin-top: 14px;
        }
    }
}
</style>
