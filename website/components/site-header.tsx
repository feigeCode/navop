import Image from "next/image";
import Link from "next/link";
import { Github, LayoutDashboard } from "lucide-react";
import { LocaleSwitcher } from "@/components/locale-switcher";
import { ThemeToggle } from "@/components/theme-toggle";
import { localePath, type Dictionary, type Locale } from "@/lib/i18n";

export function SiteHeader({
  locale,
  dict,
  active = "features"
}: {
  locale: Locale;
  dict: Dictionary;
  active?: "features" | "dashboard";
}) {
  return (
    <header className="site-header">
      <div className="container nav-row">
        <Link className="brand-link" href={localePath(locale, "/")}>
          <Image className="brand-mark" src="/brand/logo.svg" alt="" width={34} height={34} />
          <span>OnetCli</span>
        </Link>
        <nav className="nav-links" aria-label="Main">
          <a className={active === "features" ? "active" : undefined} href="#features">{dict.nav.features}</a>
          <a href="https://github.com/feigeCode/onetcli/releases">{dict.nav.download}</a>
          <Link className={active === "dashboard" ? "active" : undefined} href={localePath(locale, "/dashboard")}>{dict.nav.dashboard}</Link>
        </nav>
        <div className="nav-actions">
          <div style={{ width: 112 }}>
            <LocaleSwitcher locale={locale} />
          </div>
          <ThemeToggle />
          <a className="icon-button" href="https://github.com/feigeCode/onetcli" aria-label={dict.home.secondary}>
            <Github size={18} />
          </a>
          <Link className="button primary" href={localePath(locale, "/dashboard")}>
            <LayoutDashboard className="nav-login-icon" size={17} />
            {dict.home.primary}
          </Link>
        </div>
      </div>
    </header>
  );
}
