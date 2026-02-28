import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import zhHant from "./locales/zh-Hant.json";

const resources = {
  en: { translation: en },
  "zh-Hant": { translation: zhHant },
};

const saved = typeof localStorage !== "undefined" ? localStorage.getItem("app-locale") : null;
const fallbackLng = "en";
const lng = saved && (saved === "en" || saved === "zh-Hant") ? saved : fallbackLng;

i18n.use(initReactI18next).init({
  resources,
  lng,
  fallbackLng,
  interpolation: { escapeValue: false },
});

export default i18n;
