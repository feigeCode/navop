import { redirect } from "next/navigation";
import { SiteHeader } from "@/components/site-header";
import { SubmitButton } from "@/components/team-form";
import {
  inviteMemberAction,
  removeMemberAction,
  revokeInvitationAction,
  updateMemberRoleAction
} from "@/app/[locale]/dashboard/actions";
import { createSupabaseServerClient } from "@/lib/supabase/server";
import { getDictionary, isLocale, localePath, type Locale } from "@/lib/i18n";
import { getPendingTeamInvitations, getTeamMembers } from "@/lib/team-data";
import {
  canAssignRole,
  canManageMembers,
  canRemoveRole,
  teamRoles,
  type TeamRole
} from "@/lib/team-permissions";

export const dynamic = "force-dynamic";

export default async function TeamPage({
  params
}: {
  params: Promise<{ locale: string; teamId: string }>;
}) {
  const { locale: rawLocale, teamId } = await params;
  const locale: Locale = isLocale(rawLocale) ? rawLocale : "zh-CN";
  const dict = getDictionary(locale);
  const supabase = await createSupabaseServerClient();
  const { data } = await supabase.auth.getUser();

  if (!data.user) {
    redirect(localePath(locale, "/login"));
  }

  const [members, invitations] = await Promise.all([
    getTeamMembers(teamId),
    getPendingTeamInvitations(teamId)
  ]);
  const currentRole = members.find((member) => member.user_id === data.user.id)?.role ?? null;
  const canManage = currentRole ? canManageMembers(currentRole) : false;
  const assignableRoles = currentRole ? teamRoles.filter((role) => canAssignRole(currentRole, role)) : [];

  return (
    <div className="page-shell">
      <SiteHeader locale={locale} dict={dict} active="dashboard" />
      <main className="section">
        <div className="container stack">
          <h1 className="section-title">{dict.dashboard.members}</h1>
          <div className="panel">
            <div className="panel-header"><h2>{dict.dashboard.invite}</h2></div>
            <div className="panel-body stack">
              <RoleGuide descriptions={dict.dashboard.roleDescriptions} title={dict.dashboard.roleGuideTitle} />
              {canManage ? (
                <form className="form-grid" action={inviteMemberAction}>
                  <input type="hidden" name="locale" value={locale} />
                  <input type="hidden" name="teamId" value={teamId} />
                  <input className="input" type="email" name="email" placeholder={dict.dashboard.inviteEmail} required />
                  <select className="select" name="role" defaultValue="member">
                    {assignableRoles.map((role) => (
                      <option key={role} value={role}>{role}</option>
                    ))}
                  </select>
                  <SubmitButton>{dict.dashboard.invite}</SubmitButton>
                </form>
              ) : <p className="muted">{dict.dashboard.noManagePermission}</p>}
            </div>
          </div>
          <div className="panel">
            <div className="panel-body">
              <table className="table">
                <thead><tr><th>Email</th><th>{dict.dashboard.role}</th><th /></tr></thead>
                <tbody>
                  {members.map((member) => (
                    <tr key={member.id}>
                      <td>{member.profile?.email || member.user_id}</td>
                      <td>
                        <form action={updateMemberRoleAction} className="inline-actions">
                          <input type="hidden" name="locale" value={locale} />
                          <input type="hidden" name="teamId" value={teamId} />
                          <input type="hidden" name="memberId" value={member.id} />
                          <select
                            className="select"
                            name="role"
                            defaultValue={member.role}
                            disabled={!canUpdateMemberRole(currentRole, member.role)}
                          >
                            {teamRoles.map((role) => (
                              <option key={role} value={role} disabled={!canSelectRole(currentRole, member.role, role)}>
                                {role}
                              </option>
                            ))}
                          </select>
                          <SubmitButton>{dict.dashboard.save}</SubmitButton>
                        </form>
                      </td>
                      <td>
                        <form action={removeMemberAction}>
                          <input type="hidden" name="locale" value={locale} />
                          <input type="hidden" name="teamId" value={teamId} />
                          <input type="hidden" name="memberId" value={member.id} />
                          <button
                            className="button danger"
                            type="submit"
                            disabled={!currentRole || !canRemoveRole(currentRole, member.role)}
                          >
                            {dict.dashboard.remove}
                          </button>
                        </form>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel">
            <div className="panel-header"><h2>{dict.dashboard.pendingInvitations}</h2></div>
            <div className="panel-body">
              {invitations.length === 0 ? <p className="muted">{dict.dashboard.noPendingInvitations}</p> : (
                <table className="table">
                  <thead>
                    <tr><th>Email</th><th>{dict.dashboard.role}</th><th>{dict.dashboard.expiresAt}</th><th /></tr>
                  </thead>
                  <tbody>
                    {invitations.map((invitation) => (
                      <tr key={invitation.id}>
                        <td>{invitation.email}</td>
                        <td><span className="role-pill">{invitation.role}</span></td>
                        <td>{formatDate(invitation.expires_at, locale)}</td>
                        <td>
                          <form action={revokeInvitationAction}>
                            <input type="hidden" name="locale" value={locale} />
                            <input type="hidden" name="teamId" value={teamId} />
                            <input type="hidden" name="invitationId" value={invitation.id} />
                            <button className="button danger" type="submit" disabled={!canManage}>
                              {dict.dashboard.revoke}
                            </button>
                          </form>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}

function RoleGuide({
  descriptions,
  title
}: {
  descriptions: Record<TeamRole, string>;
  title: string;
}) {
  return (
    <div className="role-guide">
      <h3>{title}</h3>
      <div className="role-guide-grid">
        {teamRoles.map((role) => (
          <div className="role-guide-item" key={role}>
            <span className="role-pill">{role}</span>
            <p>{descriptions[role]}</p>
          </div>
        ))}
      </div>
    </div>
  );
}

function canUpdateMemberRole(actorRole: TeamRole | null, targetRole: TeamRole) {
  if (!actorRole) return false;
  if (targetRole === "owner") return actorRole === "owner";
  return canManageMembers(actorRole);
}

function canSelectRole(actorRole: TeamRole | null, targetRole: TeamRole, nextRole: TeamRole) {
  if (nextRole === targetRole) return true;
  if (!canUpdateMemberRole(actorRole, targetRole)) return false;
  return canAssignRole(actorRole!, nextRole);
}

function formatDate(value: string | null, locale: Locale) {
  if (!value) return "-";
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(value));
}
