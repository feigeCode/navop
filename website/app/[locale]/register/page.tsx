import Link from "next/link";
import { AuthShell } from "@/components/auth-shell";
import { SubmitButton } from "@/components/team-form";
import { signUpAction } from "@/app/[locale]/auth-actions";
import { getDictionary, isLocale, localePath, type Locale } from "@/lib/i18n";

export default async function RegisterPage({ params }: { params: Promise<{ locale: string }> }) {
  const { locale: rawLocale } = await params;
  const locale: Locale = isLocale(rawLocale) ? rawLocale : "zh-CN";
  const dict = getDictionary(locale);

  return (
    <AuthShell locale={locale} title={dict.auth.registerTitle} copy={dict.home.ctaCopy}>
      <form className="stack" action={signUpAction}>
        <input type="hidden" name="locale" value={locale} />
        <label className="stack">
          {dict.auth.name}
          <input className="input" name="displayName" required />
        </label>
        <label className="stack">
          {dict.auth.email}
          <input className="input" type="email" name="email" required />
        </label>
        <label className="stack">
          {dict.auth.password}
          <input className="input" type="password" name="password" required minLength={8} />
        </label>
        <SubmitButton>{dict.auth.registerButton}</SubmitButton>
        <p className="muted">
          {dict.auth.hasAccount} <Link href={localePath(locale, "/login")}>{dict.auth.goLogin}</Link>
        </p>
      </form>
    </AuthShell>
  );
}
