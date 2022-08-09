import { ulid } from "ulid";
import { NButton, NGi, NGrid, NIcon, NInput } from "naive-ui";
import { defineComponent, PropType, ref } from "vue";
import { css } from "@linaria/core";
import { CheckboxOutline, SquareOutline } from "@vicons/ionicons5";

import ExDeleteCheck from "../ExDeleteCheck";
import { HTTPRequestKVParam } from "../../commands/http_request";
import { i18nCollection } from "../../i18n";
import { padding } from "../../constants/style";

const kvClass = css`
  padding: ${padding}px;
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
  name: "APISettingParamsKeyValue",
  props: {
    params: {
      type: Array as PropType<HTTPRequestKVParam[]>,
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
      } & HTTPRequestKVParam
    ) => {
      kvList.value.push(item);
    };
    const toggleEnabled = (index: number) => {
      if (index >= kvList.value.length) {
        return;
      }
      const arr = kvList.value;
      arr[index].enabled = !arr[index].enabled;
      // TODO 更新数据
      // key/value都不为空才更新
    };
    const deleteParams = (index: number) => {
      kvList.value.splice(index, 1);
    };
    return {
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
                ></NInput>
              </NGi>
              <NGi span={12}>
                <NInput
                  placeholder={valuePlaceholder}
                  onFocus={handleFocus}
                  clearable
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
