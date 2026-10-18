// 校验 Windows 产物**内嵌**了 Common-Controls v6 清单。
//
// 为什么必须是这样：gpui 的 Windows 层静态导入 `comctl32!TaskDialogIndirect`
// （gpui_windows 的 `PlatformWindow::prompt`），而该导出只存在于 Common-Controls v6 的
// SxS 程序集。只有 exe 自己的清单声明了 `Microsoft.Windows.Common-Controls 6.0.0.0`，
// 加载器才会选 v6；否则它挂 System32 里的 5.82，进程在 `main` 之前就以
// STATUS_ENTRYPOINT_NOT_FOUND(0xC0000139) 退出，系统弹
// 「无法定位程序输入点 TaskDialogIndirect 于动态链接库 …\navop.exe 上」。
//
// 2026-10 的 v0.20.1 就是这么挂的：`gpui-fast` 的 build.rs 变成了
// `embed_resource_unless_host_provides_it()`，而 gpui-kit 声明了
// `windows-manifest-provided-by-host`（Cargo 特性只做并集、下游关不掉），宿主 navop 又
// 从来没提供清单 ⇒ 产出的 navop.exe 静态导入 comctl32 却没有清单，全部 Windows 产物不可用。
//
// 只对 navop.exe 用（WiX bundle `${PUBLIC_BASENAME}.exe` 是另一个程序，本来就没有这份清单）。
//
// 用法：node script/validate-windows-exe.mjs <navop.exe> [more exes...]

import fs from "node:fs";
import process from "node:process";

const REQUIRED_MARKERS = [
  "Microsoft.Windows.Common-Controls",
  "version=\"6.0.0.0\"",
  "publicKeyToken=\"6595b64144ccf1df\"",
];

const utf16 = (text) => Buffer.from(text, "utf16le");

/** 返回 needle 在 haystack 中的所有出现位置（同时找 ASCII 与 UTF-16LE 两种编码）。 */
function findOffsets(haystack, needle) {
  const offsets = [];
  for (const encoded of [Buffer.from(needle, "utf8"), utf16(needle)]) {
    let from = 0;
    for (;;) {
      const at = haystack.indexOf(encoded, from);
      if (at < 0) {
        break;
      }
      offsets.push(at);
      from = at + 1;
    }
  }
  return offsets;
}

/** 读取 PE 的节表；返回 { sections, resource }，非 PE 时抛错。 */
function parsePeSections(buffer, file) {
  if (buffer.length < 0x40 || buffer.toString("latin1", 0, 2) !== "MZ") {
    throw new Error(`${file} 不是 PE 文件（缺少 MZ 头）`);
  }
  const peOffset = buffer.readUInt32LE(0x3c);
  if (buffer.toString("latin1", peOffset, peOffset + 4) !== "PE\0\0") {
    throw new Error(`${file} 不是 PE 文件（缺少 PE 签名）`);
  }
  const sectionCount = buffer.readUInt16LE(peOffset + 6);
  const optionalHeaderSize = buffer.readUInt16LE(peOffset + 20);
  const sectionTable = peOffset + 24 + optionalHeaderSize;

  const sections = [];
  for (let index = 0; index < sectionCount; index += 1) {
    const at = sectionTable + index * 40;
    if (at + 40 > buffer.length) {
      throw new Error(`${file} 的节表被截断（第 ${index} 节）`);
    }
    sections.push({
      name: buffer.toString("latin1", at, at + 8).replace(/\0+$/, ""),
      rawOffset: buffer.readUInt32LE(at + 20),
      rawSize: buffer.readUInt32LE(at + 16),
    });
  }
  return { sections, resource: sections.find((section) => section.name === ".rsrc") };
}

function validate(file) {
  const buffer = fs.readFileSync(file);
  const { sections, resource } = parsePeSections(buffer, file);
  const sizeMb = (buffer.length / 1024 / 1024).toFixed(1);

  if (!resource || resource.rawSize === 0) {
    return {
      ok: false,
      reason: `没有任何资源段（.rsrc），不可能携带内嵌清单。节：${sections.map((s) => s.name || "?").join(", ")}`,
    };
  }

  const end = Math.min(resource.rawOffset + resource.rawSize, buffer.length);
  const resources = buffer.subarray(resource.rawOffset, end);

  const missing = REQUIRED_MARKERS.filter(
    (marker) => findOffsets(resources, marker).length === 0,
  );
  const importsTaskDialog = findOffsets(buffer, "TaskDialogIndirect").length > 0;

  if (missing.length === 0) {
    return {
      ok: true,
      reason: `已内嵌 Common-Controls v6 清单（.rsrc ${resource.rawSize} 字节，文件 ${sizeMb}MB${
        importsTaskDialog ? "，且静态导入了 TaskDialogIndirect" : ""
      }）`,
    };
  }

  const detail = [
    `内嵌清单缺少：${missing.map((marker) => JSON.stringify(marker)).join(", ")}`,
    importsTaskDialog
      ? "该 exe 静态导入了 comctl32!TaskDialogIndirect（gpui 的 Windows prompt），" +
        "而 Windows 只会给声明了 Common-Controls v6 依赖的进程加载 v6，所以它会在 main 之前" +
        "以 0xC0000139 退出（现象：弹「无法定位程序输入点 TaskDialogIndirect …」）。"
      : "清单不完整，comctl32 v6 的激活上下文会失效。",
    "修法：main/build.rs 里 set_version_info(VersionInfo::FILETYPE, 1) + " +
      "set_manifest(include_str!(\"../resources/windows/navop.manifest.xml\"))，" +
      "清单内容见 resources/windows/navop.manifest.xml。",
  ].join(" ");
  return { ok: false, reason: detail };
}

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("用法: node script/validate-windows-exe.mjs <navop.exe> [more exes...]");
  process.exit(2);
}

let failed = false;
for (const file of files) {
  let result;
  try {
    result = validate(file);
  } catch (error) {
    result = { ok: false, reason: error.message };
  }
  if (result.ok) {
    console.log(`OK   ${file}: ${result.reason}`);
  } else {
    failed = true;
    console.error(`FAIL ${file}: ${result.reason}`);
  }
}

process.exit(failed ? 1 : 0);
