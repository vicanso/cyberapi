import { NCard, NP, useMessage } from "naive-ui";
import { defineComponent, onBeforeMount } from "vue";
import { css } from "@linaria/core";

import ExKeyValue, {
  HandleOption,
  HandleOptionCategory,
} from "../components/ExKeyValue";
import { KVParam } from "../commands/interface";
import { useRoute } from "vue-router";
import { useEnvironmentStore } from "../stores/environment";
import { useCustomizeStore } from "../stores/variable";
import { showError } from "../helpers/util";
import { storeToRefs } from "pinia";
import ExLoading from "../components/ExLoading";
import {
  newDefaultVariable,
  Variable,
  VariableCategory,
  VariableStatus,
} from "../commands/variable";

const variableClass = css`
  .n-card__content {
    min-height: 200px;
    max-height: 400px;
    overflow-y: auto;
  }
`;

function convertKVParams(variables: Variable[]): KVParam[] {
  return variables.map((item) => {
    return {
      key: item.name,
      value: item.value,
      enabled: item.enabled == VariableStatus.Enabled,
    };
  });
}

export default defineComponent({
  name: "VariableSetting",
  props: {
    category: {
      type: String,
      required: true,
    },
    title: {
      type: String,
      required: true,
    },
    tips: {
      type: String,
      required: true,
    },
    maxWidth: {
      type: Number,
      default: () => 800,
    },
  },
  setup(props) {
    const message = useMessage();
    const route = useRoute();
    const collection = route.query.collection as string;
    let variableStore = useCustomizeStore();
    if (props.category === VariableCategory.Environment) {
      variableStore = useEnvironmentStore();
    }
    const { fetching, variables } = storeToRefs(variableStore);
    onBeforeMount(async () => {
      try {
        await variableStore.fetch(collection);
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
            const value = newDefaultVariable();
            value.collection = collection;
            value.name = item?.key || "";
            value.value = item?.value || "";
            value.enabled = enabled;
            await variableStore.add(value);
          }
          break;
        case HandleOptionCategory.Delete:
          {
            if (opt.index < variableStore.variables.length) {
              const item = variableStore.variables[opt.index];
              await variableStore.remove(item.id);
            }
          }
          break;
        default:
          {
            if (opt.index < variableStore.variables.length) {
              const updateItem = variableStore.variables[opt.index];
              const item = opt.param;
              let enabled = VariableStatus.Enabled;
              if (item && !item.enabled) {
                enabled = VariableStatus.Disabled;
              }
              updateItem.name = item?.key || "";
              updateItem.value = item?.value || "";
              updateItem.enabled = enabled;
              await variableStore.update(updateItem);
            }
          }
          break;
      }
    };
    const handleUpdate = async (params: KVParam[]) => {
      const arr = variableStore.variables.slice(0);
      const promiseList = [] as Promise<void>[];
      params.forEach((item, index) => {
        const enabled = item.enabled
          ? VariableStatus.Enabled
          : VariableStatus.Disabled;
        const value = arr[index];
        // 增加元素
        if (!value) {
          const newValue = newDefaultVariable();
          newValue.collection = collection;
          newValue.name = item.key;
          newValue.value = item.value;
          newValue.enabled = enabled;
          promiseList.push(variableStore.add(newValue));
          return;
        }
        // 其中一个不一样
        if (
          value.name !== item.key ||
          value.value !== item.value ||
          value.enabled !== enabled
        ) {
          value.name = item.key;
          value.value = item.value;
          value.enabled = enabled;
          promiseList.push(variableStore.update(value));
        }
      });
      await Promise.all(promiseList);
    };
    return {
      fetching,
      variables,
      handle,
      handleUpdate,
    };
  },
  render() {
    const { title, tips, maxWidth } = this.$props;
    const { variables, fetching } = this;
    if (fetching) {
      return <ExLoading />;
    }
    return (
      <NCard
        style={{
          maxWidth: `${maxWidth}px`,
        }}
        title={title}
        class={variableClass}
      >
        <NP>{tips}</NP>
        <ExKeyValue
          spans={[8, 16]}
          params={convertKVParams(variables)}
          onHandleParam={this.handle}
        />
      </NCard>
    );
  },
});
