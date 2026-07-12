import en from "./locales/en.json";

export type MsgKey = keyof typeof en;
export type Locale =
  | "en"
  | "ru"
  | "sr"
  | "fr"
  | "de"
  | "es"
  | "it"
  | "pl"
  | "zh"
  | "ja"
  | "ko";

export const LOCALES: { code: Locale; label: string }[] = [
  { code: "en", label: "English" },
  { code: "ru", label: "Русский" },
  { code: "sr", label: "Srpski" },
  { code: "fr", label: "Français" },
  { code: "de", label: "Deutsch" },
  { code: "es", label: "Español" },
  { code: "it", label: "Italiano" },
  { code: "pl", label: "Polski" },
  { code: "zh", label: "中文" },
  { code: "ja", label: "日本語" },
  { code: "ko", label: "한국어" },
];

const loaders: Record<Locale, () => Promise<Record<string, string>>> = {
  en: async () => en,
  ru: () => import("./locales/ru.json").then((m) => m.default),
  sr: () => import("./locales/sr.json").then((m) => m.default),
  fr: () => import("./locales/fr.json").then((m) => m.default),
  de: () => import("./locales/de.json").then((m) => m.default),
  es: () => import("./locales/es.json").then((m) => m.default),
  it: () => import("./locales/it.json").then((m) => m.default),
  pl: () => import("./locales/pl.json").then((m) => m.default),
  zh: () => import("./locales/zh.json").then((m) => m.default),
  ja: () => import("./locales/ja.json").then((m) => m.default),
  ko: () => import("./locales/ko.json").then((m) => m.default),
};

const state = $state({
  locale: "en" as Locale,
  messages: en as Record<string, string>,
});

let pluralRules = new Intl.PluralRules("en");

export async function setLocale(locale: Locale): Promise<void> {
  const messages = await loaders[locale]();
  state.locale = locale;
  state.messages = messages;
  pluralRules = new Intl.PluralRules(locale);
  document.documentElement.lang = locale;
}

export function getLocale(): Locale {
  return state.locale;
}

/** Translate a key. Plural keys use the `key_one/key_few/key_many/key_other`
 *  convention selected by Intl.PluralRules on `params.n`. */
export function t(
  key: MsgKey | (string & {}),
  params?: Record<string, string | number>,
): string {
  let template: string | undefined;
  if (params && typeof params.n === "number") {
    const category = pluralRules.select(params.n);
    template =
      state.messages[`${key}_${category}`] ??
      state.messages[`${key}_other`] ??
      state.messages[key];
  } else {
    template = state.messages[key];
  }
  // fall back to English
  template ??= (en as Record<string, string>)[key] ?? key;
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (m, name) =>
    name in params ? String(params[name]) : m,
  );
}
