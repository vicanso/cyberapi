import { defineStore } from "pinia";
import dayjs from "dayjs";

import {
  APISetting,
  listAPISetting,
  createAPISetting,
  updateAPISetting,
  deleteAPISettings,
} from "../commands/api_setting";
import { HTTPRequest } from "../commands/http_request";
import { getAPISettingStore } from "./local";
import { useEnvironmentStore, ENVRegexp } from "./environment";
import { isWebMode, setAppTitle } from "../helpers/util";
import { useGlobalReqHeaderStore } from "./global_req_header";
import { cloneDeep } from "lodash-es";

const selectedIDKey = "selectedID";

export enum SettingType {
  HTTP = "http",
  Folder = "folder",
}

export const useAPISettingStore = defineStore("apiSettings", {
  state: () => {
    return {
      selectedID: "",
      apiSettings: [] as APISetting[],
      fetching: false,
      adding: false,
      updating: false,
      removing: false,
    };
  },
  actions: {
    async setWindowTitle(id: string) {
      if (isWebMode()) {
        return;
      }
      const result = this.findByID(id);
      if (!result) {
        return;
      }

      await setAppTitle(result.name);
    },
    select(id: string) {
      // 设置失败则忽略，仅输出日志
      getAPISettingStore().setItem(selectedIDKey, id).catch(console.error);
      this.selectedID = id;
      this.setWindowTitle(this.selectedID);
    },
    getHTTPRequest(id: string) {
      const setting = this.findByID(id);
      if (!setting) {
        return {} as HTTPRequest;
      }
      return JSON.parse(setting.setting || "{}") as HTTPRequest;
    },
    fillValues(req: HTTPRequest) {
      const arr = ENVRegexp.exec(req.uri);
      if (arr?.length === 2) {
        const envValue = useEnvironmentStore().getValue(arr[1]);
        if (envValue) {
          req.uri = req.uri.replace(arr[0], envValue);
        }
      }
      const variables = useGlobalReqHeaderStore().listEnable();
      if (variables) {
        if (!req.headers) {
          req.headers = [];
        }
        variables.forEach((item) => {
          req.headers.push({
            key: item.name,
            value: item.value,
            enabled: true,
          });
        });
      }
    },
    getHTTPRequestFillValues(id: string) {
      const req = this.getHTTPRequest(id);
      const originalReq = cloneDeep(req);
      if (!req.uri) {
        return {
          originalReq,
          req,
        };
      }
      this.fillValues(req);
      return {
        originalReq,
        req,
      };
    },
    findByID(id: string): APISetting {
      const index = this.apiSettings.findIndex((item) => item.id === id);
      return this.apiSettings[index];
    },
    async updateByID(id: string, data: unknown) {
      const index = this.apiSettings.findIndex((item) => item.id === id);
      const item = Object.assign(this.apiSettings[index], data);
      await this.update(item);
    },
    async add(data: APISetting) {
      if (this.adding) {
        return;
      }
      this.adding = true;
      try {
        await createAPISetting(data);
        const arr = this.apiSettings.slice(0);
        arr.push(data);
        this.apiSettings = arr;
      } finally {
        this.adding = false;
      }
    },
    async fetch(collection: string): Promise<void> {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        // 先获取所有api setting，再获取选中id
        this.apiSettings = await listAPISetting(collection);
        this.selectedID = (await getAPISettingStore().getItem(
          selectedIDKey,
        )) as string;
        this.setWindowTitle(this.selectedID);
      } finally {
        this.fetching = false;
      }
    },
    async update(data: APISetting) {
      if (this.updating) {
        return;
      }
      this.updating = true;
      try {
        data.updatedAt = dayjs().format();
        await updateAPISetting(data);
        const arr = this.apiSettings.slice(0);
        let found = -1;
        arr.forEach((item, index) => {
          if (item.id === data.id) {
            found = index;
          }
        });
        if (found !== -1) {
          arr[found] = data;
        }
        this.apiSettings = arr;
      } finally {
        this.updating = false;
      }
    },
    async remove(id: string) {
      if (this.removing) {
        return;
      }
      this.removing = true;
      try {
        await deleteAPISettings([id]);
        this.apiSettings = this.apiSettings.filter((item) => item.id !== id);
        // 如果删除了该id，则清空
        if (id === this.selectedID) {
          this.select("");
        }
      } finally {
        this.removing = false;
      }
    },
  },
});
