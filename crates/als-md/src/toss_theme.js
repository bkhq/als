// toss-theme: per-page table of contents.
//
// Scans the rendered .content area for h2/h3 headings; when there are
// at least two (anything less is noise), inserts a small nav panel
// right after the first h1. Each heading is given a stable id when one
// isn't present so the TOC anchors work even on hand-crafted pages.
//
// Skips:
//   - pages with no `.content` (defensive),
//   - pages with fewer than 2 h2/h3 (TOC would be pointless),
//   - pages that already include a `nav.toss-toc` (author opt-out).

(function () {
  function slugify(text) {
    return text
      .toLowerCase()
      .replace(/[ \s]+/g, "-")
      .replace(/[^\w\-一-鿿]+/g, "")
      .replace(/^-+|-+$/g, "");
  }

  function build() {
    var content = document.querySelector(".content");
    if (!content) return;
    if (content.querySelector("nav.toss-toc")) return;

    var headings = content.querySelectorAll("h2, h3");
    if (headings.length < 2) return;

    var nav = document.createElement("nav");
    nav.className = "toss-toc";
    nav.setAttribute("aria-label", "On this page");
    var ol = document.createElement("ol");
    nav.appendChild(ol);

    for (var i = 0; i < headings.length; i++) {
      var h = headings[i];
      var id = h.id;
      if (!id) {
        var text = (h.textContent || "").trim();
        id = slugify(text) || "section-" + i;
        // Disambiguate if collision
        var unique = id;
        var bump = 1;
        while (document.getElementById(unique)) {
          unique = id + "-" + bump++;
        }
        h.id = unique;
        id = unique;
      }
      var li = document.createElement("li");
      li.className = "toss-toc-" + h.tagName.toLowerCase();
      var a = document.createElement("a");
      a.href = "#" + id;
      a.textContent = (h.textContent || "").trim();
      li.appendChild(a);
      ol.appendChild(li);
    }

    var firstH1 = content.querySelector("h1");
    if (firstH1 && firstH1.parentNode === content) {
      firstH1.parentNode.insertBefore(nav, firstH1.nextSibling);
    } else {
      content.insertBefore(nav, content.firstChild);
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
