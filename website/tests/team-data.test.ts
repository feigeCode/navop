import { describe, expect, test } from "vitest";
import { isMissingTeamInvitationsTableError } from "@/lib/team-data";

describe("team data errors", () => {
  test("detects missing pending invitation table errors", () => {
    expect(isMissingTeamInvitationsTableError({
      code: "PGRST205",
      message: "Could not find the table 'public.team_invitations' in the schema cache"
    })).toBe(true);
  });
});
