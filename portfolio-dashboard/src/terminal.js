"use strict";

// A small VT screen for the web terminal. The shell draws with carriage
// returns, cursor moves and erase sequences, so the page keeps a grid instead
// of appending bytes: a progress line rewrites itself, clear empties the page,
// and a full-screen program restores what it covered on the way out.
(() => {
  // Lines that scroll off the top stay readable above the live screen.
  const SCROLLBACK = 1000;

  const TABS = 8;

  class Screen {
    constructor(el, cols, rows) {
      this.el = el;
      this.cols = Math.max(cols || 80, 2);
      this.rows = Math.max(rows || 24, 2);
      this.lines = Screen.blank(this.cols, this.rows);
      this.scrollback = [];
      this.alt = null;
      this.x = 0;
      this.y = 0;
      this.top = 0;
      this.bottom = this.rows - 1;
      this.wrap = false;
      this.saved = null;
      this.paste = false;
      this.state = "text";
      this.params = "";
    }

    static blank(cols, rows) {
      return Array.from({length: rows}, () => new Array(cols).fill(" "));
    }

    write(text) {
      for (const ch of text) this.feed(ch);
    }

    // ---- parser ----------------------------------------------------------

    feed(ch) {
      switch (this.state) {
        case "text":
          this.text(ch);
          return;
        case "escape":
          this.escape(ch);
          return;
        case "csi":
          this.csi(ch);
          return;
        case "string":
          if (ch === "\x07") this.state = "text";
          else if (ch === "\x1b") this.state = "string-escape";
          return;
        case "string-escape":
          this.state = ch === "\\" ? "text" : "string";
          return;
        case "charset":
          this.state = "text";
          return;
        default:
          this.state = "text";
      }
    }

    text(ch) {
      const code = ch.codePointAt(0);
      if (code === 0x1b) {
        this.state = "escape";
      } else if (ch === "\n") {
        this.lineFeed();
      } else if (ch === "\r") {
        this.x = 0;
        this.wrap = false;
      } else if (ch === "\b") {
        this.x = Math.max(0, this.x - 1);
        this.wrap = false;
      } else if (ch === "\t") {
        this.x = Math.min(this.cols - 1, (Math.floor(this.x / TABS) + 1) * TABS);
        this.wrap = false;
      } else if (code >= 0x20 && code !== 0x7f) {
        this.print(ch);
      }
    }

    escape(ch) {
      if (ch === "[") {
        this.state = "csi";
        this.params = "";
        return;
      }
      if (ch === "]" || ch === "P" || ch === "X" || ch === "^" || ch === "_") {
        // Titles, clipboard offers and other strings are read and dropped.
        this.state = "string";
        return;
      }
      this.state = "text";
      if (ch === "7") this.save();
      else if (ch === "8") this.restore();
      else if (ch === "D") this.index();
      else if (ch === "M") this.reverseIndex();
      else if (ch === "E") {
        this.x = 0;
        this.index();
      } else if (ch === "c") this.reset();
      else if (ch === "(" || ch === ")" || ch === "*" || ch === "+" || ch === "#") this.state = "charset";
    }

    csi(ch) {
      const code = ch.charCodeAt(0);
      if (code >= 0x30 && code <= 0x3f) {
        this.params += ch;
        return;
      }
      this.state = "text";
      // Intermediate bytes ("!" " " and friends) only qualify sequences this
      // screen does not draw.
      if (code >= 0x40 && code <= 0x7e) this.apply(ch, this.params);
      this.params = "";
    }

    apply(final, raw) {
      const priv = raw.startsWith("?");
      const nums = (priv ? raw.slice(1) : raw)
        .split(";")
        .map(value => (value === "" ? 0 : Math.min(parseInt(value, 10) || 0, 9999)));
      const arg = (index, fallback) => (nums[index] === undefined || nums[index] === 0 ? fallback : nums[index]);
      switch (final) {
        case "A":
          this.y = Math.max(this.top, this.y - arg(0, 1));
          return;
        case "B":
          this.y = Math.min(this.bottom, this.y + arg(0, 1));
          return;
        case "C":
          this.x = Math.min(this.cols - 1, this.x + arg(0, 1));
          return;
        case "D":
          this.x = Math.max(0, this.x - arg(0, 1));
          return;
        case "E":
          this.x = 0;
          this.y = Math.min(this.bottom, this.y + arg(0, 1));
          return;
        case "F":
          this.x = 0;
          this.y = Math.max(this.top, this.y - arg(0, 1));
          return;
        case "G":
          this.x = Math.min(Math.max(arg(0, 1) - 1, 0), this.cols - 1);
          return;
        case "H":
        case "f":
          this.y = Math.min(Math.max(arg(0, 1) - 1, 0), this.rows - 1);
          this.x = Math.min(Math.max(arg(1, 1) - 1, 0), this.cols - 1);
          this.wrap = false;
          return;
        case "d":
          this.y = Math.min(Math.max(arg(0, 1) - 1, 0), this.rows - 1);
          this.wrap = false;
          return;
        case "e":
          this.y = Math.min(this.rows - 1, this.y + arg(0, 1));
          return;
        case "J":
          this.eraseDisplay(arg(0, 0));
          return;
        case "K":
          this.eraseLine(arg(0, 0));
          return;
        case "L":
          this.insertLines(arg(0, 1));
          return;
        case "M":
          this.deleteLines(arg(0, 1));
          return;
        case "P":
          this.deleteChars(arg(0, 1));
          return;
        case "@":
          this.insertChars(arg(0, 1));
          return;
        case "X":
          this.fill(this.x, Math.min(this.cols - 1, this.x + arg(0, 1) - 1), " ");
          return;
        case "S":
          this.scrollUp(arg(0, 1));
          return;
        case "T":
          this.scrollDown(arg(0, 1));
          return;
        case "r":
          this.region(nums);
          return;
        case "s":
          this.save();
          return;
        case "u":
          this.restore();
          return;
        case "h":
        case "l":
          this.mode(priv, nums, final === "h");
          return;
        default:
          // Colours, cursor shape, device reports: nothing to draw.
          return;
      }
    }

    mode(priv, nums, on) {
      if (!priv) return;
      for (const value of nums) {
        if (value === 1049 || value === 1047 || value === 47) {
          if (on) this.enterAlt();
          else this.leaveAlt();
        }
        // Bracketed paste: the shell can take a multi-line paste whole.
        if (value === 2004) this.paste = on;
      }
    }

    region(nums) {
      const top = (nums[0] || 1) - 1;
      const bottom = (nums[1] || this.rows) - 1;
      if (bottom - top < 1 || top < 0 || bottom >= this.rows) return;
      this.top = top;
      this.bottom = bottom;
      this.x = 0;
      this.y = top;
    }

    // ---- screen ----------------------------------------------------------

    print(ch) {
      if (this.wrap) {
        this.x = 0;
        this.lineFeed();
      }
      this.lines[this.y][this.x] = ch;
      if (this.x + 1 >= this.cols) {
        this.x = this.cols - 1;
        this.wrap = true;
      } else {
        this.x += 1;
      }
    }

    fill(from, to, ch) {
      const line = this.lines[this.y];
      for (let i = Math.max(from, 0); i <= to && i < this.cols; i += 1) line[i] = ch;
    }

    eraseLine(mode) {
      if (mode === 1) this.fill(0, this.x, " ");
      else if (mode === 2) this.fill(0, this.cols - 1, " ");
      else this.fill(this.x, this.cols - 1, " ");
      this.wrap = false;
    }

    eraseDisplay(mode) {
      if (mode === 1) {
        for (let y = 0; y < this.y; y += 1) this.lines[y].fill(" ");
        this.fill(0, this.x, " ");
      } else {
        this.fill(this.x, this.cols - 1, " ");
        for (let y = this.y + 1; y < this.rows; y += 1) this.lines[y].fill(" ");
        // The scrollback is part of what the page shows, so a clear empties it.
        if (mode >= 2) {
          this.scrollback = [];
          if (mode === 2) for (let y = 0; y <= this.y; y += 1) this.lines[y].fill(" ");
          else for (const line of this.lines) line.fill(" ");
        }
      }
      this.wrap = false;
    }

    insertChars(count) {
      const line = this.lines[this.y];
      for (let i = 0; i < count; i += 1) {
        line.splice(this.cols - 1, 1);
        line.splice(this.x, 0, " ");
      }
      this.wrap = false;
    }

    deleteChars(count) {
      const line = this.lines[this.y];
      for (let i = 0; i < count; i += 1) {
        line.splice(this.x, 1);
        line.push(" ");
      }
      this.wrap = false;
    }

    insertLines(count) {
      const limit = Math.min(count, this.bottom - this.y + 1);
      for (let i = 0; i < limit; i += 1) {
        this.lines.splice(this.bottom, 1);
        this.lines.splice(this.y, 0, new Array(this.cols).fill(" "));
      }
      this.x = 0;
      this.wrap = false;
    }

    deleteLines(count) {
      const limit = Math.min(count, this.bottom - this.y + 1);
      for (let i = 0; i < limit; i += 1) {
        this.lines.splice(this.y, 1);
        this.lines.splice(this.bottom, 0, new Array(this.cols).fill(" "));
      }
      this.x = 0;
      this.wrap = false;
    }

    scrollUp(count) {
      for (let i = 0; i < count; i += 1) {
        const line = this.lines.splice(this.top, 1)[0];
        this.lines.splice(this.bottom, 0, new Array(this.cols).fill(" "));
        if (!this.alt && this.top === 0) {
          this.scrollback.push(line.join("").replace(/\s+$/, ""));
          if (this.scrollback.length > SCROLLBACK) this.scrollback.shift();
        }
      }
      this.wrap = false;
    }

    scrollDown(count) {
      for (let i = 0; i < count; i += 1) {
        this.lines.splice(this.bottom, 1);
        this.lines.splice(this.top, 0, new Array(this.cols).fill(" "));
      }
      this.wrap = false;
    }

    index() {
      if (this.y === this.bottom) this.scrollUp(1);
      else if (this.y < this.rows - 1) this.y += 1;
      this.wrap = false;
    }

    reverseIndex() {
      if (this.y === this.top) this.scrollDown(1);
      else if (this.y > 0) this.y -= 1;
      this.wrap = false;
    }

    lineFeed() {
      this.index();
    }

    enterAlt() {
      if (this.alt) return;
      this.alt = {
        lines: this.lines,
        x: this.x,
        y: this.y,
        top: this.top,
        bottom: this.bottom,
      };
      this.lines = Screen.blank(this.cols, this.rows);
      this.x = 0;
      this.y = 0;
      this.top = 0;
      this.bottom = this.rows - 1;
      this.wrap = false;
    }

    leaveAlt() {
      if (!this.alt) return;
      this.lines = this.alt.lines;
      this.x = this.alt.x;
      this.y = this.alt.y;
      this.top = this.alt.top;
      this.bottom = this.alt.bottom;
      this.alt = null;
      this.wrap = false;
    }

    save() {
      this.saved = {x: this.x, y: this.y};
    }

    restore() {
      if (!this.saved) return;
      this.x = Math.min(this.saved.x, this.cols - 1);
      this.y = Math.min(this.saved.y, this.rows - 1);
      this.wrap = false;
    }

    reset() {
      this.lines = Screen.blank(this.cols, this.rows);
      this.scrollback = [];
      this.alt = null;
      this.x = 0;
      this.y = 0;
      this.top = 0;
      this.bottom = this.rows - 1;
      this.wrap = false;
      this.saved = null;
      this.paste = false;
      this.state = "text";
      this.params = "";
    }

    /// Where the shell left its cursor, counted in the rows as they are drawn:
    /// the scrollback above the grid comes first.
    cursor() {
      return {x: this.x, y: this.y, line: this.scrollback.length + this.y};
    }

    render() {
      if (!this.el) return;
      const lines = this.scrollback.concat(this.lines.map(line => line.join("").replace(/\s+$/, "")));
      this.el.textContent = lines.join("\n");
    }
  }

  window.WebTerm = {
    create(el, cols, rows) {
      const screen = new Screen(el, cols, rows);
      screen.render();
      return screen;
    },
  };
})();
