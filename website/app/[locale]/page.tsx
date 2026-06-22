import { SiteHeader } from "@/components/site-header";
import { MarketingHome } from "@/components/marketing-home";
import { getDictionary, isLocale, type Locale } from "@/lib/i18n";

export default async function HomePage({ params }: { params: Promise<{ locale: string }> }) {
  const { locale: rawLocale } = await params;
  const locale: Locale = isLocale(rawLocale) ? rawLocale : "zh-CN";
  const dict = getDictionary(locale);

  return (
    <div className="page-shell">
      <SiteHeader locale={locale} dict={dict} />
      <MarketingHome locale={locale} dict={dict} />
      <footer className="footer">
        <div className="container">OnetCli · GitHub Releases · Team Workspace</div>
      </footer>
    </div>
  );
}
