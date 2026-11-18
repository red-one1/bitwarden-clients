// Fork: KeePass-style auto-type of the item selected in the desktop app.

import { CipherType, FieldType } from "@bitwarden/common/vault/enums";
import { CipherView } from "@bitwarden/common/vault/models/view/cipher.view";

import {
  AUTOTYPE_SEQUENCE_FIELD,
  AutotypeSequenceError,
  AutotypeValues,
  DEFAULT_AUTOTYPE_SEQUENCE,
} from "./autotype-sequence";

/**
 * The item's sequence: its `autotype` custom field, otherwise the KeePass default for logins.
 */
export function autotypeSequenceFor(cipher: CipherView): string {
  const field = cipher.fields?.find(
    (f) => f.name?.trim().toLowerCase() === AUTOTYPE_SEQUENCE_FIELD,
  );
  const custom = field?.value?.trim();
  if (custom) {
    return custom;
  }
  if (cipher.type !== CipherType.Login) {
    throw new AutotypeSequenceError(
      `Add a custom field named "${AUTOTYPE_SEQUENCE_FIELD}" with the sequence to auto-type this item`,
    );
  }
  return DEFAULT_AUTOTYPE_SEQUENCE;
}

/**
 * The values the sequence's placeholders resolve to. Like copying, a password the user may not
 * view (and hidden custom fields of such an item) can't be auto-typed.
 */
export function autotypeValuesFor(
  cipher: CipherView,
  sequence: string,
  totp?: string,
): AutotypeValues {
  if (!cipher.viewPassword && /\{PASSWORD\}/i.test(sequence)) {
    throw new AutotypeSequenceError(
      "You can't view this item's password, so it can't be auto-typed",
    );
  }

  const fields = new Map<string, string>();
  for (const field of cipher.fields ?? []) {
    if (!field.name || field.name.trim().toLowerCase() === AUTOTYPE_SEQUENCE_FIELD) {
      continue;
    }
    if (field.type === FieldType.Hidden && !cipher.viewPassword) {
      continue;
    }
    let value: unknown = field.value;
    if (field.type === FieldType.Linked && field.linkedId != null) {
      value = cipher.linkedFieldValue(field.linkedId);
    }
    if (value != null && !fields.has(field.name)) {
      fields.set(field.name, String(value));
    }
  }

  return {
    title: cipher.name,
    username: cipher.login?.username ?? undefined,
    password: cipher.viewPassword ? (cipher.login?.password ?? undefined) : undefined,
    url: cipher.login?.uris?.find((u) => u.uri)?.uri ?? undefined,
    notes: cipher.notes ?? undefined,
    totp,
    fields,
  };
}
