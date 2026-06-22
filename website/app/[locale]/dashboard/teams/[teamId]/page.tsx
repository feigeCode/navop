import { redirect } from "next/navigation";
import { SiteHeader } from "@/components/site-header";
import { SubmitButton } from "@/components/team-form";
import {
  inviteMemberAction,
  removeMemberAction,
  updateMemberRoleAction
} from "@/app/[locale]/dashboard/actions";
import { createSupabaseServerClient } from "@/lib/supabase/server";
import { getDictionary, isLocale, localePath, type Locale } from "@/lib/i18n";
import { getTeamMembers } from "@/lib/team-data";

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

  const members = await getTeamMembers(teamId);

  return (
    <div className="page-shell">
      <SiteHeader locale={locale} dict={dict} active="dashboard" />
      <main className="section">
        <div className="container stack">
          <h1 className="section-title">{dict.dashboard.members}</h1>
          <div className="panel">
            <div className="panel-header"><h2>{dict.dashboard.invite}</h2></div>
            <form className="panel-body form-grid" action={inviteMemberAction}>
              <input type="hidden" name="locale" value={locale} />
              <input type="hidden" name="teamId" value={teamId} />
              <input className="input" type="email" name="email" placeholder={dict.dashboard.inviteEmail} required />
              <select className="select" name="role" defaultValue="member">
                <option value="member">member</option>
                <option value="admin">admin</option>
              </select>
              <SubmitButton>{dict.dashboard.invite}</SubmitButton>
            </form>
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
                          <select className="select" name="role" defaultValue={member.role}>
                            <option value="owner">owner</option>
                            <option value="admin">admin</option>
                            <option value="member">member</option>
                          </select>
                          <SubmitButton>Save</SubmitButton>
                        </form>
                      </td>
                      <td>
                        <form action={removeMemberAction}>
                          <input type="hidden" name="locale" value={locale} />
                          <input type="hidden" name="teamId" value={teamId} />
                          <input type="hidden" name="memberId" value={member.id} />
                          <button className="button danger" type="submit">Remove</button>
                        </form>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
