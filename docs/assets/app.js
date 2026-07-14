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

  var currentDict = {};

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
    currentDict = d;
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
    hideTip();
    bindTips();
  }

  /* ---- Inline tooltips (data-tip="byok" -> dict.tip_byok) ---- */
  var pop = null, popFor = null;

  /* Easter egg: the "Uh-oh!" from ICQ when you open the ICQ-era tip */
  var icqSound = null;
  function playIcqSound() {
    try {
      if (!icqSound) icqSound = new Audio("oh-oh-icq-sound.mp3");
      icqSound.currentTime = 0;
      var p = icqSound.play();
      if (p && p.catch) p.catch(function () {});
    } catch (e) {}
  }

  function ensurePop() {
    if (!pop) {
      pop = document.createElement("div");
      pop.className = "tip-pop";
      document.body.appendChild(pop);
    }
    return pop;
  }

  function hideTip() {
    if (pop) { pop.classList.remove("show"); }
    popFor = null;
  }

  function showTip(el) {
    var en = (window.I18N && window.I18N.en) || {};
    var key = "tip_" + el.getAttribute("data-tip");
    var text = currentDict[key] != null ? currentDict[key] : en[key];
    if (text == null) return;
    var p = ensurePop();
    p.innerHTML = text;
    p.classList.add("show");
    popFor = el;
    if (el.getAttribute("data-tip") === "icq") playIcqSound();
    // position below the trigger, clamped to the viewport
    var r = el.getBoundingClientRect();
    var margin = 10, width = p.offsetWidth;
    var left = r.left + window.scrollX;
    var maxLeft = window.scrollX + document.documentElement.clientWidth - width - margin;
    if (left > maxLeft) left = maxLeft;
    if (left < window.scrollX + margin) left = window.scrollX + margin;
    p.style.left = left + "px";
    p.style.top = (r.bottom + window.scrollY + 8) + "px";
  }

  function bindTips() {
    document.querySelectorAll(".tip").forEach(function (el) {
      if (el.__tipBound) return;
      el.__tipBound = true;
      el.setAttribute("role", "button");
      el.setAttribute("tabindex", "0");
      el.addEventListener("click", function (e) {
        e.stopPropagation();
        if (popFor === el && pop && pop.classList.contains("show")) hideTip();
        else showTip(el);
      });
      el.addEventListener("keydown", function (e) {
        if (e.key === "Enter" || e.key === " ") { e.preventDefault(); showTip(el); }
      });
    });
  }

  document.addEventListener("click", function (e) {
    if (pop && popFor && !popFor.contains(e.target) && !pop.contains(e.target)) hideTip();
  });
  document.addEventListener("keydown", function (e) { if (e.key === "Escape") hideTip(); });
  window.addEventListener("resize", hideTip);

  /* ---- Demo video: click to pause/resume, respect reduced motion ---- */
  function initDemoVideo() {
    var v = document.querySelector(".demo-vid");
    if (!v) return;
    var reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduce) { v.removeAttribute("autoplay"); v.pause(); }
    v.addEventListener("click", function () {
      if (v.paused) v.play(); else v.pause();
    });
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
    bindTips();
    initDemoVideo();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
