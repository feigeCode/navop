import fs from "node:fs";

const root = new URL("../", import.meta.url);

const read = (path) => fs.readFileSync(new URL(path, root), "utf8").trim();

function escapeRtf(text) {
  let escaped = "";
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index);
    if (code === 10) escaped += "\\par ";
    else if (code === 13) continue;
    else if (code === 9) escaped += "\\tab ";
    else if (code === 92 || code === 123 || code === 125) {
      escaped += `\\${text[index]}`;
    } else if (code >= 32 && code <= 126) escaped += text[index];
    else escaped += `\\u${code > 32767 ? code - 65536 : code}?`;
  }
  return escaped;
}

function writeRtf(text) {
  const output = new URL("installer/windows/navop-license.rtf", root);
  const rtf = [
    "{\\rtf1\\ansi\\ansicpg936\\deff0\\uc1",
    "{\\fonttbl{\\f0\\fnil\\fcharset134 Microsoft YaHei;}}",
    `\\viewkind4\\pard\\f0\\fs20 ${escapeRtf(text)}}`,
    "",
  ].join("\n");
  fs.writeFileSync(output, rtf, "ascii");
  console.log(`Generated ${output.pathname}`);
}

const apache = read("LICENSE-APACHE");
const supplementary = read("NAVOP_LICENSE");
const englishMarker = "\n\nThe Navop License\n\n1. Definitions";
const englishIndex = supplementary.indexOf(englishMarker);
if (englishIndex < 0) throw new Error("NAVOP_LICENSE English section not found");

const chineseSupplementary = supplementary
  .slice(0, englishIndex)
  .replace(/^The Navop License/, "Navop 补充许可协议");
const englishSupplementary = supplementary.slice(englishIndex + 2);

const license = [
  "Navop Software License Agreement / Navop 软件许可协议",
  "",
  "Installing or using Navop means that you accept all terms below.",
  "安装或使用 Navop 即表示您接受以下全部条款。",
  "",
  "Part 1: Navop Supplementary License (Chinese)",
  "第一部分：Navop 补充许可协议（中文）",
  "",
  chineseSupplementary,
  "",
  "Part 2: Navop Supplementary License (English)",
  "第二部分：Navop 补充许可协议（英文）",
  "",
  englishSupplementary,
  "",
  "Part 3: Apache License 2.0",
  "第三部分：Apache License 2.0",
  "",
  apache,
].join("\n");

writeRtf(license);
