import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export type OneHubAuthSession = {
  accessToken: string;
  refreshToken: string;
  userId: string | null;
  expiresAt: number | null;
};

type OneHubAuthJson = {
  access_token?: unknown;
  refresh_token?: unknown;
  user_id?: unknown;
  expires_at?: unknown;
};

export function getOneHubAuthPath() {
  return process.env.ONETCLI_ONE_HUB_AUTH_PATH ||
    join(homedir(), "Library", "Application Support", "one-hub", "auth.json");
}

export function readOneHubAuthSessionFromFile(path: string): OneHubAuthSession | null {
  if (!existsSync(path)) {
    return null;
  }

  let parsed: OneHubAuthJson;
  try {
    parsed = JSON.parse(readFileSync(path, "utf8")) as OneHubAuthJson;
  } catch {
    return null;
  }

  if (typeof parsed.access_token !== "string" || typeof parsed.refresh_token !== "string") {
    return null;
  }

  return {
    accessToken: parsed.access_token,
    refreshToken: parsed.refresh_token,
    userId: typeof parsed.user_id === "string" ? parsed.user_id : null,
    expiresAt: typeof parsed.expires_at === "number" ? parsed.expires_at : null
  };
}

export function readOneHubAuthSession() {
  return readOneHubAuthSessionFromFile(getOneHubAuthPath());
}

export function shouldUseOneHubAuthFallback() {
  if (process.env.NODE_ENV === "production") {
    return false;
  }
  return process.env.ONETCLI_ENABLE_ONE_HUB_AUTH !== "0" &&
    process.env.ONETCLI_ENABLE_ONE_HUB_AUTH !== "false";
}
