import { enMessages, enMetadata, type LocaleMessages } from "./en";
import { zhHantMessages, zhHantMetadata } from "./zh-Hant";

export interface LocaleMetadata {
  name: string;
  label: string;
};

export const localeRegistry = {
  en: { messages: enMessages, metadata: enMetadata },
  "zh-Hant": { messages: zhHantMessages, metadata: zhHantMetadata },
} as const;

export type Locale = keyof typeof localeRegistry;

export const SUPPORTED_LOCALES: readonly Locale[] = ["en", "zh-Hant"];

export type { LocaleMessages };
