import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
loadAncestorEnvLocal(root);
mirrorServerSupabaseEnv();

function loadAncestorEnvLocal(startDir) {
  let current = startDir;
  for (let depth = 0; depth < 4; depth += 1) {
    const envPath = join(current, ".env.local");
    if (existsSync(envPath)) {
      loadEnvFile(envPath);
    }
    const next = dirname(current);
    if (next === current) return;
    current = next;
  }
}

function loadEnvFile(path) {
  const lines = readFileSync(path, "utf8").split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const index = trimmed.indexOf("=");
    if (index < 0) continue;
    const key = trimmed.slice(0, index).trim();
    const value = stripEnvQuotes(trimmed.slice(index + 1).trim());
    if (process.env[key] === undefined) {
      process.env[key] = value;
    }
  }
}

function stripEnvQuotes(value) {
  if (
    (value.startsWith("\"") && value.endsWith("\"")) ||
    (value.startsWith("'") && value.endsWith("'"))
  ) {
    return value.slice(1, -1);
  }
  return value;
}

function mirrorServerSupabaseEnv() {
  process.env.NEXT_PUBLIC_SUPABASE_URL ??= process.env.SUPABASE_URL;
  process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY ??= process.env.SUPABASE_ANON_KEY;
}

/** @type {import('next').NextConfig} */
const nextConfig = {
  typedRoutes: false,
  turbopack: {
    root
  }
};

export default nextConfig;
