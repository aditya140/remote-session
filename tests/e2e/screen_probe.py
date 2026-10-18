"""Small screen oracle for Crossterm's cursor/erase/SGR output in keyboard proofs.

This is not the product terminal renderer. Unsupported CSI fails the proof instead
of guessing; VT100 replay/checkpoint qualification lives in the Rust adapter tests.
"""
import codecs
import unicodedata

class ScreenProbe:
    def __init__(self, rows=32, columns=120):
        self.rows, self.columns = rows, columns
        self.grid = [[" "] * columns for _ in range(rows)]
        self.x = self.y = 0
        self.saved = (0, 0)
        self.state, self.sequence = "text", ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def csi(self, final):
        private = self.sequence.startswith("?")
        values = [int(v) if v else 0 for v in self.sequence.lstrip("?").strip().split(";")]
        first = values[0] or 1
        if final in ("H", "f"):
            self.y, self.x = first - 1, (values[1] or 1) - 1 if len(values) > 1 else 0
        elif final == "G": self.x = first - 1
        elif final == "d": self.y = first - 1
        elif final == "A": self.y -= first
        elif final == "B": self.y += first
        elif final == "C": self.x += first
        elif final == "D": self.x -= first
        elif final == "E": self.y += first; self.x = 0
        elif final == "F": self.y -= first; self.x = 0
        elif final == "s": self.saved = (self.x, self.y)
        elif final == "u": self.x, self.y = self.saved
        elif final == "J":
            if values[0] in (2, 3): self.grid = [[" "] * self.columns for _ in range(self.rows)]
            elif values[0] == 0:
                self.grid[self.y][self.x:] = [" "] * (self.columns - self.x)
                for row in range(self.y + 1, self.rows): self.grid[row] = [" "] * self.columns
            else: raise AssertionError("Unsupported erase in screen probe")
        elif final == "K":
            start, end = (0, self.columns) if values[0] == 2 else ((0, self.x + 1) if values[0] == 1 else (self.x, self.columns))
            self.grid[self.y][start:end] = [" "] * (end - start)
        elif final in ("m", "h", "l", "q"):
            if private and final == "h" and values[0] == 1049:
                self.grid = [[" "] * self.columns for _ in range(self.rows)]
                self.x = self.y = 0
        else: raise AssertionError(f"Unsupported screen probe CSI {self.sequence}{final}")
        self.x = max(0, min(self.columns - 1, self.x))
        self.y = max(0, min(self.rows - 1, self.y))

    def feed(self, data):
        for character in self.decoder.decode(data):
            if self.state == "osc":
                if character == "\a": self.state = "text"
                elif character == "\x1b": self.state = "osc_escape"
                continue
            if self.state == "osc_escape": self.state = "text" if character == "\\" else "osc"; continue
            if self.state == "escape":
                if character == "[": self.state, self.sequence = "csi", ""
                elif character == "]": self.state = "osc"
                elif character in ("=", ">"): self.state = "text"
                else: raise AssertionError(f"Unsupported screen probe escape {character!r}")
                continue
            if self.state == "csi":
                if "@" <= character <= "~": self.csi(character); self.state = "text"
                else: self.sequence += character
                continue
            if character == "\x1b": self.state = "escape"
            elif character == "\r": self.x = 0
            elif character == "\n": self.y = min(self.y + 1, self.rows - 1)
            elif character == "\b": self.x = max(0, self.x - 1)
            elif character == "\t": self.x = min((self.x // 8 + 1) * 8, self.columns - 1)
            elif character >= " ":
                if unicodedata.combining(character):
                    if self.x: self.grid[self.y][self.x - 1] += character
                    continue
                if self.x >= self.columns: self.x = 0; self.y = min(self.y + 1, self.rows - 1)
                self.grid[self.y][self.x] = character
                width = 2 if unicodedata.east_asian_width(character) in ("W", "F") else 1
                if width == 2 and self.x + 1 < self.columns: self.grid[self.y][self.x + 1] = ""
                self.x += width

    def text(self):
        return "\n".join("".join(row) for row in self.grid)
