/* Skim site — language switcher & i18n runtime.
   Translations live in locales.js (window.I18N). Only elements carrying
   data-i18n / data-i18n-html are swapped, so untagged content (the English
   legal text on privacy/terms) always stays English. */
(function () {
  var LANGS = [
    ["en", "English"], ["ru", "Русский"], ["sr", "Srpski"], ["fr", "Français"],
    ["de", "Deutsch"], ["es", "Español"], ["it", "Italiano"], ["pl", "Polski"],
    ["zh", "中文"], ["ja", "日本語"], ["ko", "한국어"]
  ];

  function dict(code) {
    return (window.I18N && window.I18N[code]) || (window.I18N && window.I18N.en) || {};
  }

  function pickInitial() {
    try {
      var saved = localStorage.getItem("skim_lang");
      if (saved && window.I18N && window.I18N[saved]) return saved;
    } catch (e) {}
    var nav = (navigator.language || "en").slice(0, 2).toLowerCase();
    if (nav === "sr" || nav === "hr" || nav === "bs") nav = "sr";
    return (window.I18N && window.I18N[nav]) ? nav : "en";
  }

  function applyLang(code) {
    var d = dict(code), en = (window.I18N && window.I18N.en) || {};
    document.documentElement.lang = code;
    document.querySelectorAll("[data-i18n]").forEach(function (el) {
      var k = el.getAttribute("data-i18n");
      var v = d[k] != null ? d[k] : en[k];
      if (v != null) el.textContent = v;
    });
    document.querySelectorAll("[data-i18n-html]").forEach(function (el) {
      var k = el.getAttribute("data-i18n-html");
      var v = d[k] != null ? d[k] : en[k];
      if (v != null) el.innerHTML = v;
    });
    try { localStorage.setItem("skim_lang", code); } catch (e) {}
    var sel = document.getElementById("langSelect");
    if (sel) sel.value = code;
  }

  function init() {
    var sel = document.getElementById("langSelect");
    if (sel) {
      sel.innerHTML = LANGS.map(function (l) {
        return '<option value="' + l[0] + '">' + l[1] + "</option>";
      }).join("");
      sel.addEventListener("change", function (e) { applyLang(e.target.value); });
    }
    applyLang(pickInitial());
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
