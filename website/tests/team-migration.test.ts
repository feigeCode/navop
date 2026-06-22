import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";

const migration = readFileSync(
  new URL("../supabase/migrations/202606220001_website_team_management.sql", import.meta.url),
  "utf8"
);
const cleanupMigration = readFileSync(
  new URL("../supabase/migrations/202606220002_simplify_team_management.sql", import.meta.url),
  "utf8"
);

describe("team management migration", () => {
  test("creates only the compact team tables", () => {
    const teamsIndex = migration.indexOf("create table if not exists public.teams");
    const membersIndex = migration.indexOf("create table if not exists public.team_members");

    expect(teamsIndex).toBeGreaterThanOrEqual(0);
    expect(membersIndex).toBeGreaterThan(teamsIndex);
    expect(migration).not.toContain("create table if not exists public.profiles");
    expect(migration).not.toContain("create table if not exists public.team_invitations");
    expect(migration).not.toContain("create table if not exists public.audit_events");
  });

  test("keeps desktop team sync columns in the schema", () => {
    expect(migration).toContain("key_verification text");
    expect(migration).toContain("key_version integer");
    expect(migration).toContain("joined_at timestamptz");
  });

  test("provides an RPC for adding an existing user by email", () => {
    expect(migration).toContain("create or replace function public.add_team_member_by_email");
    expect(migration).toContain("from auth.users where lower(email) = v_email");
    expect(migration).toContain("User does not exist. Ask them to sign in first.");
  });

  test("cleans up old invitation tables for already-migrated databases", () => {
    expect(cleanupMigration).toContain("drop table if exists public.team_invitations cascade");
    expect(cleanupMigration).toContain("drop table if exists public.audit_events cascade");
    expect(cleanupMigration).toContain("drop table if exists public.profiles cascade");
    expect(cleanupMigration).toContain("create or replace function public.add_team_member_by_email");
  });
});
