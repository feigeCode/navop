import en from "@/messages/en-US.json";
import zhCN from "@/messages/zh-CN.json";
import zhTW from "@/messages/zh-TW.json";

export const locales = ["zh-CN", "en-US", "zh-TW"] as const;
export type Locale = (typeof locales)[number];
export const defaultLocale: Locale = "zh-CN";

const dictionaries = {
  "zh-CN": zhCN,
  "en-US": en,
  "zh-TW": zhTW
};

export type Dictionary = typeof zhCN;

export function isLocale(value: string | undefined): value is Locale {
  return locales.includes(value as Locale);
}

export function getDictionary(locale: Locale): Dictionary {
  return dictionaries[locale];
}

export function localePath(locale: Locale, path: string) {
  return `/${locale}${path.startsWith("/") ? path : `/${path}`}`;
}
