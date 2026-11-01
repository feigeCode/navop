# 更新镜像源（Cloudflare / CNB / GitHub）

> 状态：已实现（客户端 + CI）。相关代码：`crates/app_kit/src/update/mirror.rs`、
> `crates/app_kit/src/update/mod.rs`、`crates/core/src/settings.rs`、
> `crates/navop_app/src/settings/update_mirror.rs`。
> 通道机制见 [`beta-channel.md`](beta-channel.md)。

## 1. 要解决的问题

更新清单原先只有一条链路：Cloudflare（R2 公开域名）上的 `updates/**/latest.json`，
失败后回退 GitHub Releases API。国内网络访问 GitHub 不稳定，而 R2 域名也可能被墙，
于是「检查更新」偶发失败。这里给用户一个手动选择镜像的入口，并且**任何情况下都保留
其余镜像作为回退**，让「选错源」最多多花一次请求，不会把检查更新卡死。

## 2. 三个镜像的能力差异

| 镜像 | 数据形态 | 取版本号的方式 | 取安装包的方式 |
| --- | --- | --- | --- |
| Cloudflare | R2 上的 JSON 清单 | 直接读 `updates/**/latest.json` 的 `version` | 清单里的 `downloads` / `sha256s` |
| GitHub | Releases API | `releases/latest`（GitHub 语义**只给正式版**） | Release 资产（按 `EXPECTED_ARCHIVE_NAME` 匹配） |
| CNB | Release 资产里的 JSON 清单 | 读 `git-upload-pack` 广告里的 tag，再按通道选最大 tag | CNB Release 同名资产（把清单里的 R2 地址改写过去） |

CNB 侧有三条实测约束，直接决定了上面的实现：

1. **仓库文件直链对匿名不可用**：`/-/raw/<ref>/<path>`、`/-/blob/...`、`/-/files/...`
   对未登录请求返回 SPA HTML 或 401/400，所以清单不能放仓库文件，只能放 Release 资产。
2. **Release 下载没有 `latest` 别名**：`.../-/releases/download/latest/<asset>` 返回 404，
   必须自带 tag，因此客户端要先用 tag 列表挑出目标 tag。
3. **git smart HTTP 匿名可读**：`https://cnb.cool/navop-dev/navop.git/info/refs?service=git-upload-pack`
   直接返回 pkt-line 广告，含全部 `refs/tags/vX.Y.Z[-pre]`，这是唯一可用的 tag 发现手段。

## 3. feed 资产契约（服务端）

CNB 只同步 GitHub Release 的资产（`sync-cnb-release-assets.yml` 调 `mpgrm releases sync`），
所以更新清单必须以**扁平资产**的形式挂在 Release 上，命名规则：

- 稳定清单：`feed-<product>.json`
- 测试清单：`feed-<product>-beta.json`

`<product>` 与独立产品的 feed 段一致：全量产品是 `navop`，独立产品分别是 `navop-shell`、
`navop-data`。资产内容与 R2 上对应的 `latest.json` **逐字节相同**（同一源文件 `cp` 出来，
不重新生成），避免两份清单漂移。

发布时的一致性规则（`.github/workflows/upload-r2.yml`）：

- 稳定发布：写稳定 R2 键位 + 上传 `feed-<product>.json` 与 `feed-<product>-beta.json`，
  并把同一份稳定清单**同时写到 beta 的 R2 键位**。否则 beta 客户端会永远停在最后一个
  beta，等不到新的正式版（beta 通道从不读稳定键位）。
- 测试发布：只写 beta R2 键位 + 只上传 `feed-<product>-beta.json`，绝不碰稳定键位和
  稳定命名的资产。
- CNB 同步（`sync-cnb-release-assets.yml`）由 `upload-r2.yml` 在 feed 资产上传完成后
  派发，保证 CNB 拉到的 Release 里已经有清单。

## 4. 客户端尝试顺序与回退语义

内置顺序（`AUTO_MIRROR_ORDER`）：**Cloudflare → CNB → GitHub**。

用户显式选择某个镜像时，只把它**提前到第一位**，其余镜像保持相对顺序跟在后面，例如：

| 选择 | 实际顺序 |
| --- | --- |
| 自动 | Cloudflare → CNB → GitHub |
| Cloudflare | Cloudflare → CNB → GitHub |
| CNB | CNB → Cloudflare → GitHub |
| GitHub | GitHub → Cloudflare → CNB |

其余语义：

- **测试通道不回退 GitHub**：`releases/latest` 只给正式版，回退过去会把 beta 用户
  悄悄「降级」到稳定版提示。测试通道的顺序是 Cloudflare → CNB（两者都没有时视为没有更新）。
- **「已是最新」不早退**：某个镜像已经是最新时仍继续问后面的镜像。保留原有
  「R2 清单慢一拍、GitHub 已经发了新版本」的兜底能力。
- **全部缺失清单**（都是 404）：测试通道视为「还没有测试版」，稳定通道报
  `更新接口返回异常状态码: 404 Not Found`。
- **全部失败**（网络错误等）：报最后一条错误；没有任何可用镜像时提示「没有可用的更新源」。
- 本地清单仿真（`NAVOP_UPDATE_URL` 指向本地文件）不参与镜像链路，仍走原来的单源逻辑。

## 5. 下载地址改写

CNB 清单里的 `download_url` / `fallback_download_url` 指向 R2。命中 CNB 镜像后，客户端
把这两个地址改写成 CNB Release 上的**同名资产**：

```
https://cnb.cool/navop-dev/navop/-/releases/download/<tag>/<asset>
```

改名只取 URL 最后一段文件名（丢掉 query/fragment），安装器本来就用这个文件名，
`sha256s` 里的校验值不变。改写失败（URL 不是文件形式）时保持原地址，不影响可用性。

## 6. 设置项与持久化

- 字段：`AppSettings::update_mirror: Option<UpdateMirror>`，`None` 等价于 `Auto`。
  取值 `auto` / `cloudflare` / `cnb` / `github`（snake_case），未知取值回退 `auto`。
- UI：设置 → 关于 → 更新 → 「更新镜像源」下拉。切换后立即落盘、弹通知并按新源重查一次，
  不需要等下次启动。
- 与更新通道互不影响：通道决定读哪份清单（稳定/测试），镜像决定清单从哪个源拿。

## 7. 验证

```bash
cargo test -p app_kit --lib update          # 镜像顺序、回退链、CNB tag 解析与地址改写
cargo test -p one-core --lib update_        # UpdateChannel / UpdateMirror 持久化
cargo test -p navop_app --lib settings::update  # 设置项
cargo test -p navop_app --lib setting_tab       # 关于页分组装配
cargo test -p navop_app --test i18n_locale_keys_contract
node script/test-release-packaging.mjs      # workflow 文本断言 + prune/feed 仿真
```

手工验证 CNB 链路（匿名即可）：

```bash
curl -s "https://cnb.cool/navop-dev/navop.git/info/refs?service=git-upload-pack" | head -c 200
curl -sI "https://cnb.cool/navop-dev/navop/-/releases/download/v0.22.0/sha256sums.txt"
```

## 8. 已知限制

- 现有已发布版本不认识 CNB 源；CNB 镜像只对包含本改动的版本起效。
- 首次发布包含本改动的版本之后，需到 CNB Release 页面确认 `feed-*.json` 资产确实被
  `mpgrm releases sync` 同步过去（若它按资产名白名单过滤，需要补一条同步规则）。
- 走 CNB 时每次检查多一次 refs 请求（tag 发现），这是「没有 `latest` 别名」的代价。
- 清单版本号与包内 `sha256` 仍只由 Cloudflare 清单提供校验值；CNB 与 GitHub 的安装包
  不做签名校验（现状如此，未因本改动变化）。
