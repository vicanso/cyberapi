import { NCard, useMessage } from "naive-ui";
import { defineComponent, onBeforeMount, ref } from "vue";
import { css } from "@linaria/core";
import { ulid } from "ulid";

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
import {
  Environment,
  EnvironmentStatus,
  newDefaultEnvironment,
} from "../commands/environment";
import ExLoading from "../components/ExLoading";

const dialogClass = css`
  max-width: 800px;
`;

function convertKVParams(environments: Environment[]): KVParam[] {
  return environments.map((item) => {
    return {
      key: item.name,
      value: item.value,
      enabled: item.enabled == EnvironmentStatus.Enabled,
    };
  });
}

export default defineComponent({
  name: "EnvironmentSetting",
  setup() {
    const message = useMessage();
    const route = useRoute();
    const collection = route.query.id as string;
    const environmentStore = useEnvironmentStore();
    const { environments, fetching } = storeToRefs(environmentStore);
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
            let enabled = EnvironmentStatus.Enabled;
            if (item && !item.enabled) {
              enabled = EnvironmentStatus.Disabled;
            }
            const newEnv = newDefaultEnvironment();
            newEnv.collection = collection;
            newEnv.name = item?.key || "";
            newEnv.value = item?.value || "";
            newEnv.enabled = enabled;
            await environmentStore.add(newEnv);
          }
          break;
        case HandleOptionCategory.Delete:
          {
            if (opt.index < environmentStore.environments.length) {
              const item = environmentStore.environments[opt.index];
              await environmentStore.remove(item.id);
            }
          }
          break;
        default:
          {
            if (opt.index < environmentStore.environments.length) {
              const updateItem = environmentStore.environments[opt.index];
              const item = opt.param;
              let enabled = EnvironmentStatus.Enabled;
              if (item && !item.enabled) {
                enabled = EnvironmentStatus.Disabled;
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
      const arr = environmentStore.environments.slice(0);
      const promiseList = [] as Promise<void>[];
      params.forEach((item, index) => {
        const enabled = item.enabled
          ? EnvironmentStatus.Enabled
          : EnvironmentStatus.Disabled;
        const env = arr[index];
        // 增加元素
        if (!env) {
          const newEnv = newDefaultEnvironment();
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
      <NCard title={i18nEnvironment("title")} class={dialogClass}>
        <ExKeyValue
          spans={[10, 14]}
          params={convertKVParams(environments)}
          onHandleParam={this.handle}
        />
      </NCard>
    );
  },
});
