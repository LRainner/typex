// i18n.js — plain script, no ES module syntax
// Provides window.typexI18n for lightweight multi-language support.

window.typexI18n = {
  currentLang: 'zh-CN',
  translations: {},
  defaultTranslations: {}, // fallback language (en)

  async init(lang) {
    this.currentLang = lang || this.detectSystemLang();

    try {
      // Load default language first as fallback
      const defaultResp = await fetch('locales/en.json');
      if (defaultResp.ok) this.defaultTranslations = await defaultResp.json();

      // Load target language
      if (this.currentLang === 'en') {
        this.translations = this.defaultTranslations;
      } else {
        const resp = await fetch(`locales/${this.currentLang}.json`);
        if (resp.ok) {
          this.translations = await resp.json();
        } else {
          console.warn(`Failed to load locale ${this.currentLang}, falling back to en`);
          this.translations = this.defaultTranslations;
        }
      }
    } catch (e) {
      console.warn('i18n init failed:', e);
      this.translations = {};
    }

    this.applyAll();
  },

  t(key) {
    return this.translations[key] || this.defaultTranslations[key] || key;
  },

  detectSystemLang() {
    const sysLang = navigator.language || 'zh-CN';
    if (sysLang.startsWith('en')) return 'en';
    return 'zh-CN';
  },

  // Walk all [data-i18n] elements and update their textContent
  // Walk all [data-i18n-placeholder] elements and update their placeholder
  applyAll() {
    document.querySelectorAll('[data-i18n]').forEach(el => {
      const key = el.getAttribute('data-i18n');
      if (key) el.textContent = this.t(key);
    });
    document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
      const key = el.getAttribute('data-i18n-placeholder');
      if (key) el.placeholder = this.t(key);
    });
  },
};
