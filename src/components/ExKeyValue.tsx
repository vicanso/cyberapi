import { ulid } from "ulid";
import { NButton, NGi, NGrid, NIcon, NInput } from "naive-ui";
import { defineComponent, PropType, ref } from "vue";
import { css } from "@linaria/core";
import { CheckboxOutline, SquareOutline } from "@vicons/ionicons5";

import ExDeleteCheck from "./ExDeleteCheck";
import { KVParam } from "../commands/interface";
import { i18nCollection } from "../i18n";
import { padding } from "../constants/style";

const kvClass = css`
  .item {
    margin-bottom: ${padding}px;
  }
  .btns {
    float: right;
    .n-button {
      padding: 0 8px !important;
      margin-left: 2px;
    }
    i {
      font-size: 16px;
    }
  }
  .kv {
    margin-right: 70px;
  }
`;

export default defineComponent({
  name: "ExKeyValue",
  props: {
    params: {
      type: Array as PropType<KVParam[]>,
      required: true,
    },
    onUpdateParams: {
      type: Function as PropType<(params: KVParam[]) => void>,
      required: true,
    },
  },
  setup(props) {
    const arr = props.params.map((item) => {
      return Object.assign(
        {
          id: ulid(),
        },
        item
      );
    });
    const kvList = ref(arr);
    const addParams = (
      item: {
        id: string;
      } & KVParam
    ) => {
      kvList.value.push(item);
    };
    const handleUpdate = () => {
      if (props.onUpdateParams) {
        props.onUpdateParams(kvList.value);
      }
    };
    const toggleEnabled = (index: number) => {
      if (index >= kvList.value.length) {
        return;
      }
      const item = kvList.value[index];
      item.enabled = !item.enabled;
      if (item.key && item.value) {
        handleUpdate();
      }
    };
    const deleteParams = (index: number) => {
      kvList.value.splice(index, 1);
      handleUpdate();
    };
    return {
      handleUpdate,
      kvList,
      toggleEnabled,
      deleteParams,
      addParams,
    };
  },
  render() {
    const { kvList } = this;
    const arr = kvList.slice(0);
    const lastItem = {
      id: ulid(),
      key: "",
      value: "",
      enabled: true,
    };
    arr.push(lastItem);
    const namePlaceholder = i18nCollection("namePlaceholder");
    const valuePlaceholder = i18nCollection("valuePlaceholder");
    const size = arr.length;
    const list = arr.map((item, index) => {
      const isLast = index === size - 1;
      const handleFocus = () => {
        // 点击最后一个元素，则添加
        if (isLast) {
          this.addParams(lastItem);
        }
      };
      return (
        <div class="item" key={item.id}>
          {!isLast && (
            <div class="btns">
              <NButton
                quaternary
                onClick={() => {
                  this.toggleEnabled(index);
                }}
              >
                <NIcon>
                  {item.enabled && <CheckboxOutline />}
                  {!item.enabled && <SquareOutline />}
                </NIcon>
              </NButton>
              <ExDeleteCheck
                onConfirm={() => {
                  this.deleteParams(index);
                }}
              />
            </div>
          )}
          <div class="kv">
            <NGrid yGap={padding} xGap={padding}>
              <NGi span={12}>
                <NInput
                  placeholder={namePlaceholder}
                  onFocus={handleFocus}
                  clearable
                  defaultValue={arr[index].key}
                  onUpdateValue={(value) => {
                    arr[index].key = value;
                    this.handleUpdate();
                  }}
                ></NInput>
              </NGi>
              <NGi span={12}>
                <NInput
                  placeholder={valuePlaceholder}
                  onFocus={handleFocus}
                  clearable
                  defaultValue={arr[index].value}
                  onUpdateValue={(value) => {
                    arr[index].value = value;
                    this.handleUpdate();
                  }}
                ></NInput>
              </NGi>
            </NGrid>
          </div>
        </div>
      );
    });
    return <div class={kvClass}>{list}</div>;
  },
});
