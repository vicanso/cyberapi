import { storeToRefs } from "pinia";
import { defineComponent, PropType, ref } from "vue";
import {
  NForm,
  NFormItem,
  NInput,
  NP,
  NGrid,
  NGi,
  NDatePicker,
  useMessage,
  NButton,
} from "naive-ui";
import dayjs from "dayjs";

import { i18nCookie, i18nCommon } from "../i18n";
import { Cookie } from "../commands/cookies";
import { useCookieStore } from "../stores/cookie";
import { showError } from "../helpers/util";

export default defineComponent({
  name: "ExCookieEditor",
  props: {
    cookie: {
      type: Object as PropType<Cookie>,
      required: true,
    },
    onBack: {
      type: Function as PropType<() => void>,
      default: () => {
        console.info("back");
      },
    },
  },
  setup(props) {
    const message = useMessage();
    const cookieStore = useCookieStore();
    const updateValues = ref({
      value: "",
      expires: "",
    });
    const cookie = Object.assign({}, props.cookie);
    const update = async () => {
      const { value, expires } = updateValues.value;
      if (value === cookie.value && expires === cookie.expires) {
        message.warning(i18nCookie("modifyBeforeSubmit"));
        return;
      }
      try {
        cookie.value = value;
        cookie.expires = expires;
        await cookieStore.addOrUpdate(cookie);
        if (props.onBack) {
          props.onBack();
        }
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      updateValues,
      update,
    };
  },
  render() {
    const { cookie } = this.$props;
    let defaultExpires = null;
    if (cookie.expires) {
      defaultExpires = dayjs(cookie.expires).unix() * 1000;
    }
    return (
      <NForm>
        <NGrid xGap={15}>
          <NGi span={8}>
            <NFormItem label={i18nCookie("name")}>
              <NP>{cookie.name}</NP>
            </NFormItem>
          </NGi>
          <NGi span={8}>
            <NFormItem label={i18nCookie("path")}>
              <NP>{cookie.path}</NP>
            </NFormItem>
          </NGi>
          <NGi span={8}>
            <NFormItem label={i18nCookie("domain")}>
              <NP>{cookie.domain}</NP>
            </NFormItem>
          </NGi>
          <NGi span={8}>
            <NFormItem label={i18nCookie("value")}>
              <NInput
                defaultValue={cookie.value}
                clearable
                onUpdateValue={(value) => {
                  this.updateValues.value = value;
                }}
              />
            </NFormItem>
          </NGi>
          <NGi span={16}>
            <NFormItem label={i18nCookie("expires")}>
              <NDatePicker
                class="widthFull"
                type="datetime"
                defaultValue={defaultExpires}
                onUpdateValue={(value) => {
                  this.updateValues.expires = dayjs(value).toString();
                }}
                clearable
              />
            </NFormItem>
          </NGi>
          <NGi span={12}>
            <NButton
              class="widthFull"
              onClick={() => {
                this.update();
              }}
            >
              {i18nCommon("confirm")}
            </NButton>
          </NGi>
          <NGi span={12}>
            <NButton
              class="widthFull"
              onClick={() => {
                this.$props.onBack();
              }}
            >
              {i18nCommon("back")}
            </NButton>
          </NGi>
        </NGrid>
      </NForm>
    );
  },
});
