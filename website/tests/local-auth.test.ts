import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, test } from "vitest";
import { getSupabaseEnv } from "@/lib/env";
import {
  readOneHubAuthSessionFromFile,
  shouldUseOneHubAuthFallback
} from "@/lib/supabase/local-auth";

describe("local Supabase defaults", () => {
  test("uses local Supabase when env is not configured", () => {
    const oldUrl = process.env.NEXT_PUBLIC_SUPABASE_URL;
    const oldAnonKey = process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY;
    const oldServerUrl = process.env.SUPABASE_URL;
    const oldServerAnonKey = process.env.SUPABASE_ANON_KEY;
    delete process.env.NEXT_PUBLIC_SUPABASE_URL;
    delete process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY;
    delete process.env.SUPABASE_URL;
    delete process.env.SUPABASE_ANON_KEY;

    try {
      expect(getSupabaseEnv()).toEqual({
        url: "http://127.0.0.1:54321",
        anonKey: expect.stringContaining("eyJ")
      });
    } finally {
      if (oldUrl === undefined) delete process.env.NEXT_PUBLIC_SUPABASE_URL;
      else process.env.NEXT_PUBLIC_SUPABASE_URL = oldUrl;
      if (oldAnonKey === undefined) delete process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY;
      else process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY = oldAnonKey;
      if (oldServerUrl === undefined) delete process.env.SUPABASE_URL;
      else process.env.SUPABASE_URL = oldServerUrl;
      if (oldServerAnonKey === undefined) delete process.env.SUPABASE_ANON_KEY;
      else process.env.SUPABASE_ANON_KEY = oldServerAnonKey;
    }
  });
});

describe("one-hub auth file", () => {
  test("reads access and refresh tokens without exposing unrelated fields", () => {
    const dir = mkdtempSync(join(tmpdir(), "onetcli-auth-"));
    const file = join(dir, "auth.json");
    writeFileSync(file, JSON.stringify({
      access_token: "access-token",
      refresh_token: "refresh-token",
      user_id: "user-id",
      expires_at: 123
    }));

    try {
      expect(readOneHubAuthSessionFromFile(file)).toEqual({
        accessToken: "access-token",
        refreshToken: "refresh-token",
        userId: "user-id",
        expiresAt: 123
      });
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("returns null for missing or incomplete auth files", () => {
    const dir = mkdtempSync(join(tmpdir(), "onetcli-auth-"));
    const file = join(dir, "auth.json");
    writeFileSync(file, JSON.stringify({ access_token: "access-token" }));

    try {
      expect(readOneHubAuthSessionFromFile(join(dir, "missing.json"))).toBeNull();
      expect(readOneHubAuthSessionFromFile(file)).toBeNull();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("only enables auth file fallback outside production", () => {
    const oldNodeEnv = process.env.NODE_ENV;
    const oldEnabled = process.env.ONETCLI_ENABLE_ONE_HUB_AUTH;

    try {
      setNodeEnv("development");
      delete process.env.ONETCLI_ENABLE_ONE_HUB_AUTH;
      expect(shouldUseOneHubAuthFallback()).toBe(true);

      process.env.ONETCLI_ENABLE_ONE_HUB_AUTH = "0";
      expect(shouldUseOneHubAuthFallback()).toBe(false);

      setNodeEnv("production");
      process.env.ONETCLI_ENABLE_ONE_HUB_AUTH = "1";
      expect(shouldUseOneHubAuthFallback()).toBe(false);
    } finally {
      setNodeEnv(oldNodeEnv);
      if (oldEnabled === undefined) delete process.env.ONETCLI_ENABLE_ONE_HUB_AUTH;
      else process.env.ONETCLI_ENABLE_ONE_HUB_AUTH = oldEnabled;
    }
  });
});

function setNodeEnv(value: string | undefined) {
  const env = process.env as Record<string, string | undefined>;
  if (value === undefined) delete env.NODE_ENV;
  else env.NODE_ENV = value;
}
