// Fork: KeePass-style auto-type of the item selected in the desktop app.
//
// Parses a KeePass 2.x auto-type sequence (https://keepass.info/help/base/autotype.html) and
// resolves its placeholders into the steps the native module types. Placeholder values are typed
// literally: their characters are never interpreted as sequence syntax.

/** One resolved auto-type step; mirrors `autotype_sequence.AutotypeStep` of the native module. */
export type AutotypeStep =
  | { kind: "text"; text: string }
  | ({ kind: "key"; vk: number } & AutotypeModifiers)
  | ({ kind: "char"; text: string } & AutotypeModifiers)
  | { kind: "delay"; ms: number }
  | { kind: "keyDelay"; ms: number };

export type AutotypeModifiers = { shift?: boolean; ctrl?: boolean; alt?: boolean };

/** Item values the placeholders resolve to. Missing values type nothing, as in KeePass. */
export type AutotypeValues = {
  title?: string;
  username?: string;
  password?: string;
  url?: string;
  notes?: string;
  /** The current TOTP code; only looked up when the sequence contains `{TOTP}`. */
  totp?: string;
  /** Custom fields by name, for `{S:name}`. */
  fields?: ReadonlyMap<string, string>;
};

export const DEFAULT_AUTOTYPE_SEQUENCE = "{USERNAME}{TAB}{PASSWORD}{ENTER}";

/** Name of the custom field that holds an item's sequence (matched case-insensitively). */
export const AUTOTYPE_SEQUENCE_FIELD = "autotype";

const MAX_REPEAT = 100;

export class AutotypeSequenceError extends Error {}

/** Windows virtual key codes of the KeePass special keys. */
const SPECIAL_KEYS: ReadonlyMap<string, number> = new Map([
  ["TAB", 0x09],
  ["ENTER", 0x0d],
  ["UP", 0x26],
  ["DOWN", 0x28],
  ["LEFT", 0x25],
  ["RIGHT", 0x27],
  ["INSERT", 0x2d],
  ["INS", 0x2d],
  ["DELETE", 0x2e],
  ["DEL", 0x2e],
  ["HOME", 0x24],
  ["END", 0x23],
  ["PGUP", 0x21],
  ["PGDN", 0x22],
  ["SPACE", 0x20],
  ["BACKSPACE", 0x08],
  ["BS", 0x08],
  ["BKSP", 0x08],
  ["BREAK", 0x03],
  ["CAPSLOCK", 0x14],
  ["ESC", 0x1b],
  ["WIN", 0x5b],
  ["LWIN", 0x5b],
  ["RWIN", 0x5c],
  ["APPS", 0x5d],
  ["HELP", 0x2f],
  ["NUMLOCK", 0x90],
  ["PRTSC", 0x2c],
  ["SCROLLLOCK", 0x91],
  ["ADD", 0x6b],
  ["SUBTRACT", 0x6d],
  ["MULTIPLY", 0x6a],
  ["DIVIDE", 0x6f],
  ...Array.from({ length: 16 }, (_, i) => [`F${i + 1}`, 0x70 + i] as [string, number]),
  ...Array.from({ length: 10 }, (_, i) => [`NUMPAD${i}`, 0x60 + i] as [string, number]),
]);

/** `{+}`, `{{}`, ...: characters that have a meaning in the sequence syntax. */
const LITERAL_ESCAPES = new Set(["+", "%", "^", "~", "(", ")", "[", "]", "{", "}"]);

const PLACEHOLDERS = new Set(["TITLE", "USERNAME", "PASSWORD", "URL", "NOTES", "TOTP", "TIMEOTP"]);

/** Whether the sequence needs the TOTP code, so the caller only generates it when used. */
export function sequenceUsesTotp(sequence: string): boolean {
  return /\{(TOTP|TIMEOTP)\}/i.test(sequence);
}

/** Parses `sequence` and resolves it against `values`. Throws {@link AutotypeSequenceError}. */
export function buildAutotypeSteps(sequence: string, values: AutotypeValues): AutotypeStep[] {
  const parser = new Parser(sequence, values);
  parser.parseUntil(null, {});
  return mergeText(parser.steps);
}

class Parser {
  readonly steps: AutotypeStep[] = [];
  private pos = 0;

  constructor(
    private readonly sequence: string,
    private readonly values: AutotypeValues,
  ) {}

  /** Parses up to `end` (or the end of the sequence), applying `inherited` modifiers. */
  parseUntil(end: ")" | null, inherited: AutotypeModifiers): void {
    while (this.pos < this.sequence.length) {
      if (end != null && this.sequence[this.pos] === end) {
        this.pos++;
        return;
      }
      this.parseItem(inherited);
    }
    if (end != null) {
      throw new AutotypeSequenceError("Missing ')' in the auto-type sequence");
    }
  }

  private parseItem(inherited: AutotypeModifiers): void {
    const modifiers = { ...inherited };
    let hasModifier = false;
    while (this.pos < this.sequence.length && "+^%".includes(this.sequence[this.pos])) {
      const m = this.sequence[this.pos++];
      if (m === "+") {
        modifiers.shift = true;
      } else if (m === "^") {
        modifiers.ctrl = true;
      } else {
        modifiers.alt = true;
      }
      hasModifier = true;
    }
    if (this.pos >= this.sequence.length) {
      if (hasModifier) {
        throw new AutotypeSequenceError("The auto-type sequence ends with a modifier key");
      }
      return;
    }

    const c = this.sequence[this.pos];
    if (c === "(") {
      this.pos++;
      this.parseUntil(")", modifiers);
    } else if (c === ")") {
      throw new AutotypeSequenceError("Unexpected ')' in the auto-type sequence");
    } else if (c === "~") {
      this.pos++;
      this.key(0x0d, modifiers);
    } else if (c === "{") {
      this.parseBraces(modifiers);
    } else if (c === "}") {
      throw new AutotypeSequenceError("Unexpected '}' in the auto-type sequence; type it as {}}");
    } else {
      const ch = String.fromCodePoint(this.sequence.codePointAt(this.pos)!);
      this.pos += ch.length;
      this.char(ch, modifiers);
    }
  }

  private parseBraces(modifiers: AutotypeModifiers): void {
    // `{}}` and `{{}` contain a brace as their content.
    let close = this.sequence.indexOf("}", this.pos + 2);
    if (this.sequence[this.pos + 1] === "}" && this.sequence[this.pos + 2] === "}") {
      close = this.pos + 2;
    }
    if (close < 0) {
      throw new AutotypeSequenceError("Missing '}' in the auto-type sequence");
    }
    const content = this.sequence.slice(this.pos + 1, close);
    this.pos = close + 1;
    this.parseCode(content, modifiers);
  }

  private parseCode(content: string, modifiers: AutotypeModifiers): void {
    const upper = content.toUpperCase();

    if (upper.startsWith("S:")) {
      this.placeholderText(this.customField(content.slice(2)), modifiers);
      return;
    }
    if (PLACEHOLDERS.has(upper)) {
      this.placeholderText(this.placeholder(upper), modifiers);
      return;
    }
    if (upper === "CLEARFIELD") {
      this.key(0x24, {});
      this.key(0x23, { shift: true });
      this.key(0x2e, {});
      return;
    }

    const delayAll = /^DELAY\s*=\s*(\d+)$/.exec(upper);
    if (delayAll) {
      this.steps.push({ kind: "keyDelay", ms: Number(delayAll[1]) });
      return;
    }
    const delay = /^DELAY\s+(\d+)$/.exec(upper);
    if (delay) {
      this.steps.push({ kind: "delay", ms: Number(delay[1]) });
      return;
    }
    const vkey = /^VKEY\s+(0X[0-9A-F]+|\d+)(?:\s+([EN]*))?$/.exec(upper);
    if (vkey) {
      const vk = Number(vkey[1]);
      if (vk < 1 || vk > 0xfe) {
        throw new AutotypeSequenceError(`Invalid virtual key {${content}}`);
      }
      this.key(vk, modifiers);
      return;
    }

    // `{TAB 3}`, `{+ 2}`: a key or escaped character repeated.
    const repeat = /^(.+?)\s+(\d+)$/.exec(content);
    const name = repeat ? repeat[1] : content;
    const count = repeat ? Number(repeat[2]) : 1;
    if (count > MAX_REPEAT) {
      throw new AutotypeSequenceError(`Too many repetitions in {${content}}`);
    }

    const vk = SPECIAL_KEYS.get(name.toUpperCase());
    if (vk != null) {
      for (let i = 0; i < count; i++) {
        this.key(vk, modifiers);
      }
      return;
    }
    if (LITERAL_ESCAPES.has(name)) {
      for (let i = 0; i < count; i++) {
        this.char(name, modifiers);
      }
      return;
    }

    throw new AutotypeSequenceError(`Unsupported code {${content}} in the auto-type sequence`);
  }

  private placeholder(name: string): string | undefined {
    switch (name) {
      case "TITLE":
        return this.values.title;
      case "USERNAME":
        return this.values.username;
      case "PASSWORD":
        return this.values.password;
      case "URL":
        return this.values.url;
      case "NOTES":
        return this.values.notes;
      default:
        return this.values.totp;
    }
  }

  private customField(name: string): string {
    const fields = this.values.fields ?? new Map<string, string>();
    const exact = fields.get(name);
    if (exact != null) {
      return exact;
    }
    for (const [fieldName, value] of fields) {
      if (fieldName.toLowerCase() === name.toLowerCase()) {
        return value;
      }
    }
    throw new AutotypeSequenceError(`The item has no custom field "${name}"`);
  }

  private placeholderText(value: string | undefined, modifiers: AutotypeModifiers): void {
    if (!value) {
      return;
    }
    if (hasModifiers(modifiers)) {
      for (const ch of value) {
        this.char(ch, modifiers);
      }
    } else {
      this.steps.push({ kind: "text", text: value });
    }
  }

  private char(ch: string, modifiers: AutotypeModifiers): void {
    if (hasModifiers(modifiers)) {
      this.steps.push({ kind: "char", text: ch, ...modifiers });
    } else {
      this.steps.push({ kind: "text", text: ch });
    }
  }

  private key(vk: number, modifiers: AutotypeModifiers): void {
    this.steps.push({ kind: "key", vk, ...modifiers });
  }
}

function hasModifiers(m: AutotypeModifiers): boolean {
  return !!(m.shift || m.ctrl || m.alt);
}

function mergeText(steps: AutotypeStep[]): AutotypeStep[] {
  const merged: AutotypeStep[] = [];
  for (const step of steps) {
    const last = merged[merged.length - 1];
    if (step.kind === "text" && last?.kind === "text") {
      merged[merged.length - 1] = { kind: "text", text: last.text + step.text };
    } else {
      merged.push(step);
    }
  }
  return merged;
}
