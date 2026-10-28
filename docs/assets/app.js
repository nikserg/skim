/* Skim site — URL-based language switcher, tooltip pop-up, demo video UX.

   Pages are pre-rendered per language at build time (scripts/build-site.mjs),
   so each language has its own URL (/, /ru/, /es/, etc.) and canonical. This
   runtime still:
     - renders the <select id="langSelect">, with onchange = navigate to the
       equivalent URL in the chosen language;
     - fills any content whose translation was not present at pre-render time
       (data-i18n / data-i18n-html), as a safety net;
     - powers inline tooltips (data-tip) and the click-to-pause demo video. */
(function () {
  var LANGS = [
    ["en", "English"], ["ru", "Русский"], ["sr", "Srpski"], ["fr", "Français"],
    ["de", "Deutsch"], ["es", "Español"], ["it", "Italiano"], ["pl", "Polski"],
    ["zh", "中文"], ["ja", "日本語"], ["ko", "한국어"]
  ];
  var LANG_CODES = LANGS.map(function (l) { return l[0]; });

  /* Detect the active language from the URL. The landing lives at `/` for
     English and `/<lang>/` otherwise; blog pages at `/blog/...` and
     `/<lang>/blog/...`. */
  function currentLang() {
    var p = window.location.pathname;
    var m = p.match(/^\/([a-z]{2})(\/|$)/);
    if (m && LANG_CODES.indexOf(m[1]) !== -1) return m[1];
    return "en";
  }

  /* Build the URL for the same page in a different language. */
  function urlFor(lang) {
    var p = window.location.pathname;
    var tail = p.replace(/^\/[a-z]{2}(\/|$)/, "/");
    if (tail === "" || tail === "/") tail = "/";
    return lang === "en" ? tail : "/" + lang + tail;
  }

  function dict(code) {
    return (window.I18N && window.I18N[code]) || (window.I18N && window.I18N.en) || {};
  }

  /* Safety net: fill any element whose pre-rendered text is still the English
     default (data-i18n keys present, but value hasn't been swapped). */
  function fillMissing(code) {
    var d = dict(code), en = (window.I18N && window.I18N.en) || {};
    document.documentElement.lang = code;
    document.querySelectorAll("[data-i18n]").forEach(function (el) {
      var k = el.getAttribute("data-i18n");
      var v = d[k] != null ? d[k] : en[k];
      if (v != null && el.textContent !== v) el.textContent = v;
    });
    document.querySelectorAll("[data-i18n-html]").forEach(function (el) {
      var k = el.getAttribute("data-i18n-html");
      var v = d[k] != null ? d[k] : en[k];
      if (v != null && el.innerHTML.indexOf(v) === -1) el.innerHTML = v;
    });
  }

  /* ---- Inline tooltips (data-tip="byok" -> dict.tip_byok) ---- */
  var pop = null, popFor = null;
  var icqSound = null;
  function playIcqSound() {
    try {
      if (!icqSound) icqSound = new Audio("/oh-oh-icq-sound.mp3");
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
  function hideTip() { if (pop) pop.classList.remove("show"); popFor = null; }
  function showTip(el) {
    var currentDict = dict(currentLang());
    var en = (window.I18N && window.I18N.en) || {};
    var key = "tip_" + el.getAttribute("data-tip");
    var text = currentDict[key] != null ? currentDict[key] : en[key];
    if (text == null) return;
    var p = ensurePop();
    p.innerHTML = text;
    p.classList.add("show");
    popFor = el;
    if (el.getAttribute("data-tip") === "icq") playIcqSound();
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
    v.addEventListener("click", function () { if (v.paused) v.play(); else v.pause(); });
  }

  function init() {
    var cur = currentLang();
    var sel = document.getElementById("langSelect");
    if (sel) {
      sel.innerHTML = LANGS.map(function (l) {
        return '<option value="' + l[0] + '"' + (l[0] === cur ? ' selected' : '') + '>' + l[1] + "</option>";
      }).join("");
      sel.addEventListener("change", function (e) {
        window.location.href = urlFor(e.target.value);
      });
    }
    fillMissing(cur);
    bindTips();
    initDemoVideo();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
