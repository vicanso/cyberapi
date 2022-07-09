// API应用配置下拉

import { defineComponent, PropType, ref } from "vue";
import { NButton, NDropdown, NIcon } from "naive-ui";
import { css } from "@linaria/core";
import {
  CaretDownOutline,
  CopyOutline,
  TrashOutline,
  SettingsOutline,
} from "@vicons/ionicons5";

import { SettingType } from "../../stores/api_setting";
import { i18nCommon } from "../../i18n";

const dropdownClass = css`
  .n-dropdown-option-body {
    padding: 0 5px;
  }
`;

const buttonVisibleClass = css`
  display: inline-flex !important;
`;

export enum DropdownHandleType {
  Duplicate = "duplicate",
  Delete = "delete",
  Settings = "settings",
}

export default defineComponent({
  name: "APISettingSuffixDropdown",
  props: {
    id: {
      type: String,
      required: true,
    },
    category: {
      type: String,
      default: () => SettingType.HTTP,
    },
    onSelect: {
      type: Function as PropType<(category: string) => void>,
      required: true,
    },
  },
  setup(props) {
    const visiable = ref(false);
    return {
      options: [
        {
          label: () => i18nCommon("duplicate"),
          key: DropdownHandleType.Duplicate,
          icon: () => (
            <NIcon>
              <CopyOutline />
            </NIcon>
          ),
        },
        {
          label: i18nCommon("delete"),
          key: DropdownHandleType.Delete,
          icon: () => (
            <NIcon>
              <TrashOutline />
            </NIcon>
          ),
        },
        {
          type: "divider",
          key: "d1",
        },
        {
          label: i18nCommon("settings"),
          key: DropdownHandleType.Settings,
          icon: () => (
            <NIcon>
              <SettingsOutline />
            </NIcon>
          ),
        },
      ],
      visiable,
    };
  },
  render() {
    const { options, visiable } = this;
    return (
      <NDropdown
        class={dropdownClass}
        trigger="click"
        options={options}
        onSelect={(key: string) => {
          this.visiable = false;
          this.$props.onSelect(key);
        }}
        onClickoutside={() => {
          this.visiable = false;
        }}
      >
        <NButton
          quaternary
          class={visiable && buttonVisibleClass}
          onClick={(e) => {
            this.visiable = true;
            e.preventDefault();
          }}
        >
          <NIcon>
            <CaretDownOutline />
          </NIcon>
        </NButton>
      </NDropdown>
    );
  },
});
