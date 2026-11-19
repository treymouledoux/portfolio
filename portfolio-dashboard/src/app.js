"use strict";

(() => {
  const views = {
    overview: document.getElementById("view-overview"),
    messages: document.getElementById("view-messages"),
  };
  const tabs = Array.from(document.querySelectorAll("#dashboard-tabs a[data-view]"));
  const badge = document.getElementById("inbox-badge");
  const denied = document.getElementById("tpl-denied");
  const titles = {overview: "Overview · Tailnet dashboard", messages: "Messages · Tailnet dashboard"};
  let loaded = null;
  let inflight = null;

  // Only the two dashboard views are handled here; every other link navigates.
  const scoped = pathname => pathname === "/" || pathname === "/messages";

  function focusHeading(view) {
    const heading = view?.querySelector("h1");
    if (!heading) return;
    heading.setAttribute("tabindex", "-1");
    heading.focus({preventScroll: true});
  }

  function markCurrent(name) {
    document.title = titles[name];
    for (const tab of tabs) {
      if (tab.dataset.view === name) tab.setAttribute("aria-current", "page");
      else tab.removeAttribute("aria-current");
    }
    for (const [key, view] of Object.entries(views)) {
      if (view) view.hidden = key !== name;
    }
  }

  function updateBadge(inboxView) {
    if (!badge) return;
    const unread = Number(inboxView?.dataset.unread);
    const total = Number(inboxView?.dataset.total);
    const shown = Number.isSafeInteger(unread) && unread > 0;
    badge.hidden = !shown;
    badge.textContent = shown ? (unread > 99 ? "99+" : String(unread)) : "";
    if (shown) {
      const label = `${unread} unread of ${total} total`;
      badge.setAttribute("aria-label", label);
      badge.title = label;
    } else {
      badge.removeAttribute("aria-label");
      badge.removeAttribute("title");
    }
  }

  function initWidgets() {
    window.PasskeyPanel?.init();
    window.MessageInbox?.init();
    window.HoldDelete?.init();
  }

  function notice(text) {
    const paragraph = document.createElement("p");
    paragraph.className = "notice-panel";
    paragraph.textContent = text;
    const retry = document.createElement("a");
    retry.href = "/messages";
    retry.textContent = "Try again";
    retry.dataset.retry = "true";
    paragraph.append(" ", retry);
    return paragraph;
  }

  function showOverview({push = false, replace = false, focus = false} = {}) {
    if (push) history.pushState({view: "overview"}, "", "/");
    else if (replace) history.replaceState({view: "overview"}, "", "/");
    loaded = null;
    markCurrent("overview");
    if (focus) focusHeading(views.overview);
  }

  function showDenied() {
    if (!views.messages) return;
    views.messages.replaceChildren(denied.content.cloneNode(true));
    loaded = null;
    updateBadge(null);
    markCurrent("messages");
    focusHeading(views.messages);
  }

  async function loadMessages(target, {push = false, replace = false, focus = false, force = false} = {}) {
    const url = target instanceof URL ? target : new URL(target, location.origin);
    const key = url.pathname + url.search;
    const same = key === loaded;
    if (push && !same) history.pushState({view: "messages"}, "", key);
    else if (replace) history.replaceState({view: "messages"}, "", key);
    markCurrent("messages");
    if (focus) focusHeading(views.messages);
    if (same && !force) {
      updateBadge(views.messages.querySelector("#inbox-view"));
      return;
    }
    inflight?.abort();
    const controller = new AbortController();
    inflight = controller;
    views.messages.setAttribute("aria-busy", "true");
    try {
      const response = await fetch("/messages/view" + url.search, {
        cache: "no-store", mode: "same-origin", credentials: "same-origin",
        redirect: "error", headers: {Accept: "text/html"}, signal: controller.signal,
      });
      if (response.status === 401 || response.status === 403) {
        showDenied();
        return;
      }
      if (!response.ok) throw Error(String(response.status));
      views.messages.innerHTML = await response.text();
      loaded = key;
      updateBadge(views.messages.querySelector("#inbox-view"));
      initWidgets();
    } catch (error) {
      if (error.name === "AbortError") return;
      views.messages.replaceChildren(notice("Couldn’t load messages. Check your tailnet connection and try again."));
      loaded = null;
      updateBadge(null);
    } finally {
      if (inflight === controller) inflight = null;
      views.messages.removeAttribute("aria-busy");
    }
  }

  function navigate(href, options = {}) {
    const url = new URL(href, location.origin);
    if (url.pathname === "/") showOverview(options);
    else if (url.pathname === "/messages") loadMessages(url, options);
    else location.assign(url.href);
  }

  document.addEventListener("click", event => {
    if (event.defaultPrevented || event.button !== 0) return;
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const link = event.target instanceof Element ? event.target.closest("a[href]") : null;
    if (!(link instanceof HTMLAnchorElement)) return;
    if (link.target || link.hasAttribute("download") || link.origin !== location.origin) return;
    if (!scoped(link.pathname)) return;
    event.preventDefault();
    navigate(link.href, {push: true, focus: true, force: link.dataset.retry === "true"});
  });

  window.addEventListener("popstate", () => {
    navigate(location.pathname + location.search, {focus: true});
  });

  // Messages mutations re-render in place; the server decides locked or denied.
  window.Dashboard = {
    refreshMessages({reset = false} = {}) {
      const current = location.pathname === "/messages" ? location.pathname + location.search : "/messages";
      loadMessages(reset ? "/messages" : current, {replace: true, force: true});
    },
  };

  if (views.messages && !views.messages.hidden) {
    loaded = location.pathname + location.search;
    markCurrent("messages");
    updateBadge(views.messages.querySelector("#inbox-view"));
  } else {
    showOverview();
  }
  initWidgets();
})();
