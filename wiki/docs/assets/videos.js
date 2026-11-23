/* Short demo videos (<video class="only-light|only-dark" data-clip>): they play by themselves like an animated picture, but
   only while they are on screen and visible (the hidden light/dark twin and videos scrolled out of view stay paused and are
   not downloaded), and never when the reader asked the system for less motion (prefers-reduced-motion: reduce): then the
   poster and the controls are shown and the reader decides. No library, no request to another site. */
(function () {
  "use strict";
  var quiet = window.matchMedia("(prefers-reduced-motion: reduce)");
  var seen = typeof IntersectionObserver === "function" ? new IntersectionObserver(function (entries) {
    entries.forEach(function (entry) {
      var v = entry.target;
      if (entry.isIntersecting && !quiet.matches) {
        v.preload = "auto";
        var p = v.play();
        if (p && p.catch) { p.catch(function () { /* the browser refused: the controls are there */ }); }
      } else if (!v.paused) {
        v.pause();
      }
    });
  }, { threshold: 0.35 }) : null;

  function start() {
    if (!seen) { return; }
    document.querySelectorAll("video[data-clip]").forEach(function (v) {
      if (!v.__clip) { v.__clip = true; seen.observe(v); }
    });
  }

  quiet.addEventListener && quiet.addEventListener("change", function () {
    if (quiet.matches) {
      document.querySelectorAll("video[data-clip]").forEach(function (v) { v.pause(); });
    } else {
      document.querySelectorAll("video[data-clip]").forEach(function (v) { v.__clip = false; seen && seen.unobserve(v); });
      start();
    }
  });

  if (window.document$ && window.document$.subscribe) {
    window.document$.subscribe(start);   /* instant navigation replaces the page without a reload */
  } else if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
}());
