import { NCard, NP, useMessage } from "naive-ui";
import { defineComponent, onBeforeMount } from "vue";
import { css } from "@linaria/core";

import ExKeyValue, {
  HandleOption,
  HandleOptionCategory,
} from "../components/ExKeyValue";
import { i18nEnvironment } from "../i18n";
import { KVParam } from "../commands/interface";
import { useRoute } from "vue-router";
import { useEnvironmentStore } from "../stores/environment";
import { showError } from "../helpers/util";
import { storeToRefs } from "pinia";
import ExLoading from "../components/ExLoading";
import {
  newDefaultVariable,
  Variable,
  VariableStatus,
} from "../commands/variable";

const environmentClass = css`
  max-width: 800px;
  .n-card__content {
    min-height: 200px;
    max-height: 400px;
    overflow-y: auto;
  }
`;

function convertKVParams(environments: Variable[]): KVParam[] {
  return environments.map((item) => {
    return {
      key: item.name,
      value: item.value,
      enabled: item.enabled == VariableStatus.Enabled,
    };
  });
}

export default defineComponent({
  name: "EnvironmentSetting",
  setup() {
    const message = useMessage();
    const route = useRoute();
    const collection = route.query.collection as string;
    const environmentStore = useEnvironmentStore();
    const { fetching } = storeToRefs(environmentStore);
    const environments = storeToRefs(environmentStore).variables;
    onBeforeMount(async () => {
      try {
        await environmentStore.fetch(collection);
      } catch (err) {
        showError(message, err);
      }
    });
    const handle = async (opt: HandleOption) => {
      switch (opt.category) {
        case HandleOptionCategory.Add:
          {
            const item = opt.param;
            let enabled = VariableStatus.Enabled;
            if (item && !item.enabled) {
              enabled = VariableStatus.Disabled;
            }
            const newEnv = newDefaultVariable();
            newEnv.collection = collection;
            newEnv.name = item?.key || "";
            newEnv.value = item?.value || "";
            newEnv.enabled = enabled;
            await environmentStore.add(newEnv);
          }
          break;
        case HandleOptionCategory.Delete:
          {
            if (opt.index < environmentStore.variables.length) {
              const item = environmentStore.variables[opt.index];
              await environmentStore.remove(item.id);
            }
          }
          break;
        default:
          {
            if (opt.index < environmentStore.variables.length) {
              const updateItem = environmentStore.variables[opt.index];
              const item = opt.param;
              let enabled = VariableStatus.Enabled;
              if (item && !item.enabled) {
                enabled = VariableStatus.Disabled;
              }
              updateItem.name = item?.key || "";
              updateItem.value = item?.value || "";
              updateItem.enabled = enabled;
              await environmentStore.update(updateItem);
            }
          }
          break;
      }
    };
    const handleUpdate = async (params: KVParam[]) => {
      const arr = environmentStore.variables.slice(0);
      const promiseList = [] as Promise<void>[];
      params.forEach((item, index) => {
        const enabled = item.enabled
          ? VariableStatus.Enabled
          : VariableStatus.Disabled;
        const env = arr[index];
        // 增加元素
        if (!env) {
          const newEnv = newDefaultVariable();
          newEnv.collection = collection;
          newEnv.name = item.key;
          newEnv.value = item.value;
          newEnv.enabled = enabled;
          promiseList.push(environmentStore.add(newEnv));
          return;
        }
        // 其中一个不一样
        if (
          env.name !== item.key ||
          env.value !== item.value ||
          env.enabled !== enabled
        ) {
          env.name = item.key;
          env.value = item.value;
          env.enabled = enabled;
          promiseList.push(environmentStore.update(env));
        }
      });
      await Promise.all(promiseList);
    };
    return {
      fetching,
      environments,
      handle,
      handleUpdate,
    };
  },
  render() {
    const { environments, fetching } = this;
    if (fetching) {
      return <ExLoading />;
    }
    return (
      <NCard title={i18nEnvironment("title")} class={environmentClass}>
        <NP>{i18nEnvironment("tips")}</NP>
        <ExKeyValue
          spans={[10, 14]}
          params={convertKVParams(environments)}
          onHandleParam={this.handle}
        />
      </NCard>
    );
  },
});
