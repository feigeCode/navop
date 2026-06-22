"use server";

import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import { createSupabaseServerClient } from "@/lib/supabase/server";
import { isLocale, localePath, type Locale } from "@/lib/i18n";
import { isTeamRole, type TeamRole } from "@/lib/team-permissions";

function readLocale(formData: FormData): Locale {
  const locale = String(formData.get("locale") ?? "zh-CN");
  return isLocale(locale) ? locale : "zh-CN";
}

function readRole(formData: FormData): TeamRole {
  const role = String(formData.get("role") ?? "member");
  return isTeamRole(role) ? role : "member";
}

export async function createTeamAction(formData: FormData) {
  const locale = readLocale(formData);
  const name = String(formData.get("name") ?? "");
  const description = String(formData.get("description") ?? "");
  const supabase = await createSupabaseServerClient();
  const { data, error } = await supabase.rpc("create_team_with_owner", {
    p_name: name,
    p_description: description || null
  });

  if (error) {
    redirect(`${localePath(locale, "/dashboard")}?error=${encodeURIComponent(error.message)}`);
  }
  revalidatePath(localePath(locale, "/dashboard"));
  redirect(localePath(locale, `/dashboard/teams/${data}`));
}

export async function inviteMemberAction(formData: FormData) {
  const locale = readLocale(formData);
  const teamId = String(formData.get("teamId") ?? "");
  const email = String(formData.get("email") ?? "");
  const role = readRole(formData);
  const supabase = await createSupabaseServerClient();
  const { error } = await supabase.rpc("invite_team_member", {
    p_team_id: teamId,
    p_email: email,
    p_role: role
  });

  if (error) {
    redirect(`${localePath(locale, `/dashboard/teams/${teamId}`)}?error=${encodeURIComponent(error.message)}`);
  }
  revalidatePath(localePath(locale, `/dashboard/teams/${teamId}`));
}

export async function updateMemberRoleAction(formData: FormData) {
  const locale = readLocale(formData);
  const teamId = String(formData.get("teamId") ?? "");
  const memberId = String(formData.get("memberId") ?? "");
  const role = readRole(formData);
  const supabase = await createSupabaseServerClient();
  const { error } = await supabase.rpc("update_team_member_role", {
    p_member_id: memberId,
    p_role: role
  });

  if (error) {
    redirect(`${localePath(locale, `/dashboard/teams/${teamId}`)}?error=${encodeURIComponent(error.message)}`);
  }
  revalidatePath(localePath(locale, `/dashboard/teams/${teamId}`));
}

export async function removeMemberAction(formData: FormData) {
  const locale = readLocale(formData);
  const teamId = String(formData.get("teamId") ?? "");
  const memberId = String(formData.get("memberId") ?? "");
  const supabase = await createSupabaseServerClient();
  const { error } = await supabase.rpc("remove_team_member", {
    p_member_id: memberId
  });

  if (error) {
    redirect(`${localePath(locale, `/dashboard/teams/${teamId}`)}?error=${encodeURIComponent(error.message)}`);
  }
  revalidatePath(localePath(locale, `/dashboard/teams/${teamId}`));
}

export async function revokeInvitationAction(formData: FormData) {
  const locale = readLocale(formData);
  const teamId = String(formData.get("teamId") ?? "");
  const invitationId = String(formData.get("invitationId") ?? "");
  const supabase = await createSupabaseServerClient();
  const { error } = await supabase.rpc("revoke_team_invitation", {
    p_invitation_id: invitationId
  });

  if (error) {
    redirect(`${localePath(locale, `/dashboard/teams/${teamId}`)}?error=${encodeURIComponent(error.message)}`);
  }
  revalidatePath(localePath(locale, `/dashboard/teams/${teamId}`));
}
