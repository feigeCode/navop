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
  email: string;
  role: TeamRole;
  joined_at: string | null;
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
  email: string;
  role: TeamRole;
  joined_at: string | null;
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
    .select("id,user_id,email,role,joined_at")
    .eq("team_id", teamId)
    .order("joined_at", { ascending: true });

  if (error) {
    throw error;
  }

  const rows = (data ?? []) as MemberRow[];
  return rows.map((row) => ({
    id: row.id,
    user_id: row.user_id,
    email: row.email,
    role: row.role,
    joined_at: row.joined_at
  })) as TeamMemberView[];
}
