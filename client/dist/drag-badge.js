// Drag-carry badge (shell window "drag-badge"). The shell sets the label via
// `window.__setBadge(text)`; the first label arrives in the `t` query
// parameter because the window may still be loading when it is created.
(function () {
  var el = document.getElementById("t");
  window.__setBadge = function (text) {
    el.textContent = String(text || "");
  };
  try {
    var t = new URLSearchParams(location.search).get("t");
    if (t) window.__setBadge(t);
  } catch (_) {}
})();
