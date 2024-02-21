import { ulid } from "ulid";
import { NButton, NGi, NGrid, NIcon, NInput, useMessage } from "naive-ui";
import { defineComponent, PropType, ref } from "vue";
import { css } from "@linaria/core";
import {
  CheckboxOutline,
  DocumentOutline,
  SquareOutline,
} from "@vicons/ionicons5";
import { debounce } from "lodash-es";
import { open } from "@tauri-apps/api/dialog";
import { downloadDir } from "@tauri-apps/api/path";

import ExDeleteCheck from "./ExDeleteCheck";
import { KVParam } from "../commands/interface";
import { i18nCollection, i18nCommon } from "../i18n";
import { padding } from "../constants/style";
import { showError } from "../helpers/util";

const kvClass = css`
  .item {
    margin-bottom: ${padding}px;
  }
  .n-input--autosize {
    width: 100%;
  }
  .btns {
    float: right;
    margin-left: 5px;
    .n-button {
      padding: 0 8px !important;
      margin-left: 2px;
    }
    i {
      font-size: 16px;
    }
  }
  .kv {
    margin-right: 75px;
    &.withFile {
      margin-right: 110px;
    }
  }
`;

export enum HandleOptionCategory {
  Update = "update",
  Add = "add",
  Delete = "delete",
}
export interface HandleOption {
  category: string;
  index: number;
  param?: KVParam;
  params: KVParam[];
}

type KVItem = {
  id: string;
  isNew: boolean;
} & KVParam;

export default defineComponent({
  name: "ExKeyValue",
  props: {
    params: {
      type: Array as PropType<KVParam[]>,
      required: true,
    },
    spans: {
      type: Array as PropType<number[]>,
      default: () => [12, 12],
    },
    onHandleParam: {
      type: Function as PropType<(opt: HandleOption) => void>,
      required: true,
    },
    typeList: {
      type: Array as PropType<("textarea" | "text" | "password")[]>,
      default: () => ["textarea", "textarea"],
    },
    supportFileSelect: {
      type: Boolean,
      default: () => false,
    },
  },
  setup(props) {
    const message = useMessage();
    const arr = props.params.map((item) => {
      return Object.assign(
        {
          id: ulid(),
          isNew: false,
        },
        item,
      );
    });
    const kvList = ref(arr as KVItem[]);
    const addParams = (item: KVItem) => {
      kvList.value.push(item);
    };
    const handle = (opt: HandleOption) => {
      if (props.onHandleParam) {
        props.onHandleParam(opt);
      }
    };
    const toggleEnabled = (index: number) => {
      if (index >= kvList.value.length) {
        return;
      }
      const item = kvList.value[index];
      item.enabled = !item.enabled;
      if (item.key && item.value) {
        handle({
          category: HandleOptionCategory.Update,
          param: item,
          index,
          params: kvList.value,
        });
      }
    };

    const handleUpdate = (index: number) => {
      if (index >= kvList.value.length) {
        return;
      }
      const item = kvList.value[index];
      let category = HandleOptionCategory.Update;
      if (item.isNew) {
        category = HandleOptionCategory.Add;
        item.isNew = false;
      }
      handle({
        category,
        param: item,
        index,
        params: kvList.value,
      });
    };
    const deleteParams = (index: number) => {
      const items = kvList.value.splice(index, 1);
      //  如果是新元素未添加至数据库的，则忽略
      if (items.length && items[0].isNew) {
        return;
      }

      handle({
        category: HandleOptionCategory.Delete,
        index,
        params: kvList.value,
      });
    };
    const selectFile = async (index: number) => {
      if (index >= kvList.value.length) {
        return;
      }
      try {
        const selected = await open({
          title: i18nCommon("selectFile"),
          multiple: false,
          defaultPath: await downloadDir(),
        });
        if (selected) {
          const item = kvList.value[index];
          item.value = ("file://" + selected) as string;
          item.id = ulid();
          kvList.value[index] = item;
        }
        handleUpdate(index);
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      kvList,
      handleUpdate,
      selectFile,
      toggleEnabled,
      deleteParams,
      addParams,
    };
  },
  render() {
    const { spans, typeList, supportFileSelect } = this.$props;
    const { kvList } = this;
    const arr = kvList.slice(0);
    const lastItem: KVItem = {
      id: ulid(),
      key: "",
      value: "",
      enabled: true,
      isNew: true,
    };
    arr.push(lastItem);
    const namePlaceholder = i18nCollection("namePlaceholder");
    const valuePlaceholder = i18nCollection("valuePlaceholder");
    const size = arr.length;
    const inputDebounce = 200;
    const kvCls = ["kv"];
    if (supportFileSelect) {
      kvCls.push("withFile");
    }
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
              {supportFileSelect && (
                <NButton
                  quaternary
                  onClick={() => {
                    this.selectFile(index);
                  }}
                >
                  <NIcon>
                    <DocumentOutline />
                  </NIcon>
                </NButton>
              )}
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
          <div class={kvCls}>
            <NGrid yGap={padding} xGap={padding}>
              <NGi span={spans[0] || 12}>
                <NInput
                  type={typeList[0]}
                  autosize={true}
                  placeholder={namePlaceholder}
                  onFocus={handleFocus}
                  clearable
                  defaultValue={arr[index].key}
                  onUpdateValue={debounce((value) => {
                    arr[index].key = value;
                    this.handleUpdate(index);
                  }, inputDebounce)}
                ></NInput>
              </NGi>
              <NGi span={spans[1] || 12}>
                <NInput
                  type={typeList[1]}
                  autosize={true}
                  placeholder={valuePlaceholder}
                  onFocus={handleFocus}
                  showPasswordOn="click"
                  clearable
                  defaultValue={arr[index].value}
                  onUpdateValue={debounce((value) => {
                    arr[index].value = value;
                    this.handleUpdate(index);
                  }, inputDebounce)}
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
