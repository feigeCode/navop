import { createSupabaseServerClient } from "@/lib/supabase/server";
import type { TeamRole } from "@/lib/team-permissions";

export type TeamSummary = {
  id: string;
  name: string;
  description: string | null;
  owner_id: string;
  updated_at: string | null;
  role: TeamRole;
};

export type TeamMemberView = {
  id: string;
  user_id: string;
  role: TeamRole;
  joined_at: string | null;
  profile: {
    email: string | null;
    display_name: string | null;
    avatar_url: string | null;
  } | null;
};

export type TeamInvitationView = {
  id: string;
  email: string;
  role: TeamRole;
  expires_at: string | null;
  created_at: string | null;
};

type MembershipRow = {
  role: TeamRole;
  teams: {
    id: string;
    name: string;
    description: string | null;
    owner_id: string;
    updated_at: string | null;
  } | null;
};

type MemberRow = {
  id: string;
  user_id: string;
  role: TeamRole;
  joined_at: string | null;
};

type ProfileRow = {
  id: string;
  email: string | null;
  display_name: string | null;
  avatar_url: string | null;
};

type InvitationRow = {
  id: string;
  email: string;
  role: TeamRole;
  expires_at: string | null;
  created_at: string | null;
};

export async function listCurrentUserTeams(userId: string) {
  const supabase = await createSupabaseServerClient();
  const { data, error } = await supabase
    .from("team_members")
    .select("role, teams(id,name,description,owner_id,updated_at)")
    .eq("user_id", userId)
    .order("joined_at", { ascending: true });

  if (error) {
    throw error;
  }

  return ((data ?? []) as unknown as MembershipRow[])
    .filter((row) => row.teams)
    .map((row) => ({ ...row.teams!, role: row.role }));
}

export async function getTeamMembers(teamId: string) {
  const supabase = await createSupabaseServerClient();
  const { data, error } = await supabase
    .from("team_members")
    .select("id,user_id,role,joined_at")
    .eq("team_id", teamId)
    .order("joined_at", { ascending: true });

  if (error) {
    throw error;
  }

  const rows = (data ?? []) as MemberRow[];
  const userIds = rows.map((row) => row.user_id);
  const { data: profiles } = await supabase
    .from("profiles")
    .select("id,email,display_name,avatar_url")
    .in("id", userIds);
  const profileRows = (profiles ?? []) as ProfileRow[];
  const profileMap = new Map(profileRows.map((profile) => [profile.id, profile]));

  return rows.map((row) => ({
    id: row.id,
    user_id: row.user_id,
    role: row.role,
    joined_at: row.joined_at,
    profile: profileMap.get(row.user_id) ?? null
  })) as TeamMemberView[];
}

export async function getPendingTeamInvitations(teamId: string) {
  const supabase = await createSupabaseServerClient();
  const { data, error } = await supabase
    .from("team_invitations")
    .select("id,email,role,expires_at,created_at")
    .eq("team_id", teamId)
    .is("accepted_at", null)
    .is("revoked_at", null)
    .order("created_at", { ascending: false });

  if (error) {
    if (isMissingTeamInvitationsTableError(error)) {
      return [];
    }
    throw error;
  }

  return (data ?? []) as InvitationRow[];
}

export function isMissingTeamInvitationsTableError(error: unknown) {
  if (!error || typeof error !== "object") {
    return false;
  }
  const candidate = error as { code?: unknown; message?: unknown };
  return candidate.code === "PGRST205" &&
    typeof candidate.message === "string" &&
    candidate.message.includes("team_invitations");
}
