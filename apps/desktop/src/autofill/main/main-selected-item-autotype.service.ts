// Fork: KeePass-style auto-type of the item selected in the desktop app.

import { ipcMain } from "electron";

import { autotype_sequence } from "@bitwarden/desktop-napi";
import { LogService } from "@bitwarden/logging";

import { SELECTED_ITEM_AUTOTYPE_IPC_CHANNELS } from "../models/ipc-channels";
import { AutotypeStep } from "../selected-item-autotype/autotype-sequence";

/**
 * Types the selected item into the window used before Bitwarden: the native module switches to
 * the next window below Bitwarden (as KeePass does), waits for the shortcut's modifier keys to be
 * released, then sends the steps.
 */
export class MainSelectedItemAutotypeService {
  private running = false;

  constructor(private logService: LogService) {
    ipcMain.handle(
      SELECTED_ITEM_AUTOTYPE_IPC_CHANNELS.PERFORM,
      async (_event, steps: AutotypeStep[]): Promise<string | null> => {
        if (process.platform !== "win32") {
          return "Auto-type is only supported on Windows";
        }
        if (this.running) {
          return "Auto-type is already running";
        }
        this.running = true;
        try {
          await autotype_sequence.perform(steps);
          return null;
        } catch (e) {
          const message = e instanceof Error ? e.message : String(e);
          // The message never contains item values, only what went wrong.
          this.logService.error("Selected item auto-type failed: " + message);
          return message;
        } finally {
          this.running = false;
        }
      },
    );
  }

  dispose() {
    ipcMain.removeHandler(SELECTED_ITEM_AUTOTYPE_IPC_CHANNELS.PERFORM);
  }
}
