import { getMessages, resolveLanguage } from './i18n';
import { useAppStore } from '../store/useAppStore';
import type { LanguagePreference } from '../types/api';

/**
 * Returns the resolved UI locale and typed message catalog for the current language setting.
 */
export function useI18n() {
  const pref = useAppStore((s) => s.settings.language);
  const lang = resolveLanguage(pref);
  return {
    lang,
    pref: (pref ?? 'system') as LanguagePreference,
    t: getMessages(lang),
  };
}
