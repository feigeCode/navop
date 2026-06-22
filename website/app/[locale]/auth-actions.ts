"use server";

import { redirect } from "next/navigation";
import { headers } from "next/headers";
import { createSupabaseServerClient } from "@/lib/supabase/server";
import { getSiteUrl } from "@/lib/env";
import { isLocale, localePath, type Locale } from "@/lib/i18n";

function formLocale(formData: FormData): Locale {
  const locale = String(formData.get("locale") ?? "zh-CN");
  return isLocale(locale) ? locale : "zh-CN";
}

export async function signInAction(formData: FormData) {
  const locale = formLocale(formData);
  const email = String(formData.get("email") ?? "");
  const password = String(formData.get("password") ?? "");
  const supabase = await createSupabaseServerClient();
  const { error } = await supabase.auth.signInWithPassword({ email, password });

  if (error) {
    redirect(`${localePath(locale, "/login")}?error=${encodeURIComponent(error.message)}`);
  }
  redirect(localePath(locale, "/dashboard"));
}

export async function signUpAction(formData: FormData) {
  const locale = formLocale(formData);
  const email = String(formData.get("email") ?? "");
  const password = String(formData.get("password") ?? "");
  const displayName = String(formData.get("displayName") ?? "");
  const origin = (await headers()).get("origin") ?? getSiteUrl();
  const supabase = await createSupabaseServerClient();

  const { error } = await supabase.auth.signUp({
    email,
    password,
    options: {
      data: { display_name: displayName },
      emailRedirectTo: `${origin}${localePath(locale, "/auth/callback")}`
    }
  });

  if (error) {
    redirect(`${localePath(locale, "/register")}?error=${encodeURIComponent(error.message)}`);
  }
  redirect(`${localePath(locale, "/login")}?notice=check-email`);
}

export async function signOutAction(formData: FormData) {
  const locale = formLocale(formData);
  const supabase = await createSupabaseServerClient();
  await supabase.auth.signOut();
  redirect(localePath(locale, "/"));
}
