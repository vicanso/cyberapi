import { getVersion } from "@tauri-apps/api/app";
import { message } from "@tauri-apps/api/dialog";
import dayjs from "dayjs";
import { ulid } from "ulid";

import { isWebMode } from "../helpers/util";
import {
  cmdAddVersion,
  cmdExportTables,
  cmdGetLatestVersion,
  cmdImportTables,
  cmdInitTables,
  run,
} from "./invoke";

export interface Version {
  [key: string]: unknown;
  id: string;
  version: string;
  createdAt: string;
  updatedAt: string;
}

async function getDatabaseLatestVersion() {
  if (isWebMode()) {
    return {} as Version;
  }
  return await run<Version>(cmdGetLatestVersion, {});
}

// handleDatabaseCompatible 处理数据库兼容
export async function handleDatabaseCompatible() {
  if (isWebMode()) {
    return;
  }
  try {
    await run(cmdInitTables);
    const version = await getVersion();
    const latestVersion = await getDatabaseLatestVersion();
    if (!latestVersion || latestVersion.version !== version) {
      await run(cmdAddVersion, {
        version: {
          id: ulid(),
          version,
          createdAt: dayjs().format(),
          updatedAt: dayjs().format(),
        },
      });
    }
    // TODO 后续针对数据库做更新
  } catch (err) {
    if (err instanceof Error) {
      message(err.message);
    }
    console.error(err);
  }
}

export async function exportTables(): Promise<string> {
  return await run(cmdExportTables);
}

export async function importTables(file: string) {
  return await run(cmdImportTables, {
    file,
  });
}
