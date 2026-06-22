"use client";

import { usePathname, useRouter } from "next/navigation";
import { isLocale, locales, type Locale } from "@/lib/i18n";

export function LocaleSwitcher({ locale }: { locale: Locale }) {
  const pathname = usePathname();
  const router = useRouter();

  function changeLocale(nextLocale: string) {
    const parts = pathname.split("/");
    if (isLocale(parts[1])) {
      parts[1] = nextLocale;
    }
    router.push(parts.join("/") || `/${nextLocale}`);
  }

  return (
    <select className="select" value={locale} onChange={(event) => changeLocale(event.target.value)}>
      {locales.map((item) => (
        <option key={item} value={item}>
          {item}
        </option>
      ))}
    </select>
  );
}
