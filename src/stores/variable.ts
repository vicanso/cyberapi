import dayjs from "dayjs";
import { sortBy } from "lodash-es";
import { defineStore } from "pinia";
import {
  createVariable,
  deleteVariable,
  listVariable,
  updateVariable,
  Variable,
  VariableCategory,
  VariableStatus,
} from "../commands/variable";

export const useCustomizeStore = newVariableStore(
  "customizeVariables",
  VariableCategory.Customize,
);

export function newVariableStore(name: string, category: string) {
  return defineStore(name, {
    state: () => {
      return {
        variables: [] as Variable[],
        fetching: false,
        adding: false,
        updating: false,
        removing: false,
      };
    },
    actions: {
      getValue(name: string) {
        const value = this.variables.find((item) => {
          return item.enabled === VariableStatus.Enabled && item.name === name;
        });
        return value?.value;
      },
      async add(value: Variable) {
        if (this.adding) {
          return;
        }
        this.adding = true;
        try {
          value.category = category;
          await createVariable(value);
          this.variables.push(value);
        } finally {
          this.adding = false;
        }
      },
      async fetch(collection: string) {
        if (this.fetching) {
          return;
        }
        this.fetching = true;
        try {
          const result = await listVariable(collection, category);
          this.variables = sortBy(result, (item) => item.name);
        } finally {
          this.fetching = false;
        }
      },
      listEnable(): Variable[] {
        return this.variables.filter(
          (item) => item.enabled === VariableStatus.Enabled,
        );
      },
      async update(value: Variable) {
        if (this.updating) {
          return;
        }
        this.updating = true;
        try {
          value.updatedAt = dayjs().format();
          value.category = category;
          await updateVariable(value);
          const arr = this.variables.slice(0);
          let found = -1;
          arr.forEach((item, index) => {
            if (item.id === value.id) {
              found = index;
            }
          });
          if (found !== -1) {
            arr[found] = value;
          }
          this.variables = arr;
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
          await deleteVariable([id]);
          this.variables = this.variables.filter((item) => item.id !== id);
        } finally {
          this.removing = false;
        }
      },
    },
  });
}
