import {
  AutotypeSequenceError,
  buildAutotypeSteps,
  DEFAULT_AUTOTYPE_SEQUENCE,
  sequenceUsesTotp,
} from "./autotype-sequence";

const TAB = 0x09;
const ENTER = 0x0d;

describe("buildAutotypeSteps", () => {
  const values = {
    title: "Router",
    username: "admin",
    password: "p{TAB}+^~%(x)",
    url: "https://router.home.arpa",
    notes: "line1\nline2",
    totp: "123456",
    fields: new Map([
      ["PIN", "4711"],
      ["autotype", "{USERNAME}{ENTER}"],
    ]),
  };

  it("resolves the default sequence", () => {
    expect(buildAutotypeSteps(DEFAULT_AUTOTYPE_SEQUENCE, values)).toEqual([
      { kind: "text", text: "admin" },
      { kind: "key", vk: TAB },
      { kind: "text", text: "p{TAB}+^~%(x)" },
      { kind: "key", vk: ENTER },
    ]);
  });

  it("is case-insensitive and types placeholder values literally", () => {
    expect(buildAutotypeSteps("{password}{Tab}{url}{totp}{TimeOtp}", values)).toEqual([
      { kind: "text", text: "p{TAB}+^~%(x)" },
      { kind: "key", vk: TAB },
      { kind: "text", text: "https://router.home.arpa123456123456" },
    ]);
  });

  it("types nothing for empty values", () => {
    expect(buildAutotypeSteps("{USERNAME}x{NOTES}", { username: "" })).toEqual([
      { kind: "text", text: "x" },
    ]);
  });

  it("applies modifiers to the next key, character or group", () => {
    expect(buildAutotypeSteps("^a%{F4}+(bc)^+{END}", values)).toEqual([
      { kind: "char", text: "a", ctrl: true },
      { kind: "key", vk: 0x73, alt: true },
      { kind: "char", text: "b", shift: true },
      { kind: "char", text: "c", shift: true },
      { kind: "key", vk: 0x23, ctrl: true, shift: true },
    ]);
  });

  it("handles escapes, ~ and repetitions", () => {
    expect(buildAutotypeSteps("{{}{}}{+}{^}{~}{(}{)}{%}~{TAB 3}{+ 2}", values)).toEqual([
      { kind: "text", text: "{}+^~()%" },
      { kind: "key", vk: ENTER },
      { kind: "key", vk: TAB },
      { kind: "key", vk: TAB },
      { kind: "key", vk: TAB },
      { kind: "text", text: "++" },
    ]);
  });

  it("supports delays, CLEARFIELD and VKEY", () => {
    expect(
      buildAutotypeSteps("{DELAY 500}{DELAY=50}{CLEARFIELD}{VKEY 0x41}{VKEY 13 E}", values),
    ).toEqual([
      { kind: "delay", ms: 500 },
      { kind: "keyDelay", ms: 50 },
      { kind: "key", vk: 0x24 },
      { kind: "key", vk: 0x23, shift: true },
      { kind: "key", vk: 0x2e },
      { kind: "key", vk: 0x41 },
      { kind: "key", vk: 13 },
    ]);
  });

  it("resolves custom fields by exact, then case-insensitive name", () => {
    expect(buildAutotypeSteps("{S:PIN}{s:pin}", values)).toEqual([
      { kind: "text", text: "47114711" },
    ]);
  });

  it.each([
    ["{S:missing}", /no custom field "missing"/],
    ["{APPACTIVATE Notepad}", /Unsupported code/],
    ["{NOPE}", /Unsupported code/],
    ["{TAB", /Missing '}'/],
    ["+(ab", /Missing '\)'/],
    ["ab)", /Unexpected '\)'/],
    ["a}", /Unexpected '}'/],
    ["abc^", /ends with a modifier/],
    ["{TAB 101}", /Too many repetitions/],
    ["{VKEY 300}", /Invalid virtual key/],
  ])("rejects %s", (sequence, message) => {
    expect(() => buildAutotypeSteps(sequence, values)).toThrow(AutotypeSequenceError);
    expect(() => buildAutotypeSteps(sequence, values)).toThrow(message);
  });
});

describe("sequenceUsesTotp", () => {
  it("detects TOTP placeholders", () => {
    expect(sequenceUsesTotp("{USERNAME}{tab}{Totp}")).toBe(true);
    expect(sequenceUsesTotp("{TIMEOTP}")).toBe(true);
    expect(sequenceUsesTotp(DEFAULT_AUTOTYPE_SEQUENCE)).toBe(false);
  });
});
