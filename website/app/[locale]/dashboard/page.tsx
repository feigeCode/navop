import Link from "next/link";
import { redirect } from "next/navigation";
import { SiteHeader } from "@/components/site-header";
import { SubmitButton } from "@/components/team-form";
import { createTeamAction } from "@/app/[locale]/dashboard/actions";
import { createSupabaseServerClient } from "@/lib/supabase/server";
import { getDictionary, isLocale, localePath, type Locale } from "@/lib/i18n";
import { listCurrentUserTeams } from "@/lib/team-data";

export const dynamic = "force-dynamic";

export default async function DashboardPage({ params }: { params: Promise<{ locale: string }> }) {
  const { locale: rawLocale } = await params;
  const locale: Locale = isLocale(rawLocale) ? rawLocale : "zh-CN";
  const dict = getDictionary(locale);
  const supabase = await createSupabaseServerClient();
  const { data } = await supabase.auth.getUser();

  if (!data.user) {
    redirect(localePath(locale, "/login"));
  }

  const teams = await listCurrentUserTeams(data.user.id);

  return (
    <div className="page-shell">
      <SiteHeader locale={locale} dict={dict} active="dashboard" />
      <main className="section">
        <div className="container dashboard-grid">
          <aside className="panel">
            <div className="panel-header"><h2>{dict.dashboard.createTeam}</h2></div>
            <form className="panel-body stack" action={createTeamAction}>
              <input type="hidden" name="locale" value={locale} />
              <label className="stack">{dict.dashboard.teamName}<input className="input" name="name" required /></label>
              <label className="stack">{dict.dashboard.description}<textarea className="textarea" name="description" /></label>
              <SubmitButton>{dict.dashboard.createTeam}</SubmitButton>
            </form>
          </aside>
          <section className="stack">
            <h1 className="section-title">{dict.dashboard.title}</h1>
            <p className="section-copy">{dict.dashboard.subtitle}</p>
            {teams.length === 0 ? <p className="muted">{dict.dashboard.empty}</p> : null}
            {teams.map((team) => (
              <Link className="card" key={team.id} href={localePath(locale, `/dashboard/teams/${team.id}`)}>
                <h3>{team.name}</h3>
                <p>{team.description || team.id}</p>
                <span className="role-pill">{team.role}</span>
              </Link>
            ))}
          </section>
        </div>
      </main>
    </div>
  );
}
