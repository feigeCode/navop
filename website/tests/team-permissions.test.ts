import { describe, expect, test } from "vitest";
import {
  canAssignRole,
  canManageMembers,
  canManageOwners,
  canRemoveRole
} from "@/lib/team-permissions";

describe("team permissions", () => {
  test("owner can manage members and owners", () => {
    expect(canManageMembers("owner")).toBe(true);
    expect(canManageOwners("owner")).toBe(true);
  });

  test("admin can manage non-owner members only", () => {
    expect(canManageMembers("admin")).toBe(true);
    expect(canManageOwners("admin")).toBe(false);
    expect(canRemoveRole("admin", "member")).toBe(true);
    expect(canRemoveRole("admin", "owner")).toBe(false);
  });

  test("member cannot assign roles", () => {
    expect(canAssignRole("member", "admin")).toBe(false);
    expect(canAssignRole("member", "member")).toBe(false);
  });
});
