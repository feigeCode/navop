import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const docsSiteRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = dirname(docsSiteRoot);
const outputDir = join(docsSiteRoot, "docs", "public", "images");
const screenshots = [
  "app.png",
  "database.png",
  "ssh.png",
  "sftp.png",
  "sftp_sidebar.png",
  "remote_file_editor.png",
  "redis.png",
  "mongodb.png",
  "chatdb.png",
  "monitor.png",
  "er.png",
  "extension.png"
];

mkdirSync(outputDir, { recursive: true });

for (const screenshot of screenshots) {
  copyFileSync(join(repositoryRoot, screenshot), join(outputDir, screenshot));
}

console.log(`Synced ${screenshots.length} documentation screenshots.`);
