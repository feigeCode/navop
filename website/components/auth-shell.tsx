import Link from "next/link";
import { localePath, type Locale } from "@/lib/i18n";

export function AuthShell({
  children,
  locale,
  title,
  copy
}: {
  children: React.ReactNode;
  locale: Locale;
  title: string;
  copy: string;
}) {
  return (
    <main className="auth-layout">
      <section className="auth-visual">
        <h1>{title}</h1>
        <p>{copy}</p>
        <Link className="button" href={localePath(locale, "/")}>OnetCli</Link>
      </section>
      <section className="auth-panel">{children}</section>
    </main>
  );
}
