import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";

const migration = readFileSync(
  new URL("../supabase/migrations/202606220001_website_team_management.sql", import.meta.url),
  "utf8"
);

describe("team management migration", () => {
  test("creates base team tables before dependent tables", () => {
    const teamsIndex = migration.indexOf("create table if not exists public.teams");
    const membersIndex = migration.indexOf("create table if not exists public.team_members");
    const invitationsIndex = migration.indexOf("create table if not exists public.team_invitations");

    expect(teamsIndex).toBeGreaterThanOrEqual(0);
    expect(membersIndex).toBeGreaterThan(teamsIndex);
    expect(invitationsIndex).toBeGreaterThan(membersIndex);
  });

  test("keeps desktop team sync columns in the schema", () => {
    expect(migration).toContain("key_verification text");
    expect(migration).toContain("key_version integer");
    expect(migration).toContain("joined_at timestamptz");
  });

  test("provides an RPC for revoking pending invitations", () => {
    expect(migration).toContain("create or replace function public.revoke_team_invitation");
    expect(migration).toContain("set revoked_at = now()");
  });
});
