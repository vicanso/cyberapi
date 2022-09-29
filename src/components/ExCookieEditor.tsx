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
  FormInst,
} from "naive-ui";
import dayjs from "dayjs";

import { i18nCookie, i18nCommon } from "../i18n";
import { Cookie } from "../commands/cookies";
import { useCookieStore } from "../stores/cookie";
import { newRequireRules, showError } from "../helpers/util";

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
    const formRef = ref<FormInst | null>(null);

    const updateValues = ref({
      name: props.cookie.name,
      path: props.cookie.path,
      domain: props.cookie.domain,
      value: props.cookie.value,
      expires: props.cookie.expires,
    });
    const cookie = Object.assign({}, props.cookie);
    const update = async () => {
      const { value, expires, name, path, domain } = updateValues.value;
      try {
        await new Promise((resolve, reject) => {
          if (!formRef.value) {
            return reject(new Error("form ref is null"));
          }
          formRef.value.validate((errors) => {
            if (!errors) {
              resolve(null);
              return;
            }
            reject(errors[0][0]);
          });
        });
        cookie.value = value;
        cookie.expires = expires;
        // 新增
        if (!cookie.name) {
          cookie.name = name;
          cookie.path = path || "/";
          cookie.domain = domain || "";
        }
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
      formRef,
    };
  },
  render() {
    const { cookie } = this.$props;
    let defaultExpires = null;
    if (cookie.expires) {
      defaultExpires = dayjs(cookie.expires).unix() * 1000;
    }
    const shortcuts = {
      [i18nCookie("neverExpired")]: 32503651200000,
    };

    const isAdd = !cookie.name;
    const rules = newRequireRules([
      "name",
      "value",
      "path",
      "domain",
      "expires",
    ]);
    return (
      <NForm ref="formRef" rules={rules} model={this.updateValues}>
        <NGrid xGap={15}>
          <NGi span={8}>
            <NFormItem label={i18nCookie("name")} path="name">
              {!isAdd && <NP>{cookie.name}</NP>}
              {isAdd && (
                <NInput
                  placeholder={i18nCookie("namePlaceholder")}
                  clearable
                  onUpdateValue={(value) => {
                    this.updateValues.name = value;
                  }}
                />
              )}
            </NFormItem>
          </NGi>
          <NGi span={8}>
            <NFormItem label={i18nCookie("path")} path="path">
              {!isAdd && <NP>{cookie.path}</NP>}
              {isAdd && (
                <NInput
                  placeholder={i18nCookie("pathPlaceholder")}
                  clearable
                  onUpdateValue={(value) => {
                    this.updateValues.path = value;
                  }}
                />
              )}
            </NFormItem>
          </NGi>
          <NGi span={8}>
            <NFormItem label={i18nCookie("domain")} path="domain">
              {!isAdd && <NP>{cookie.domain}</NP>}
              {isAdd && (
                <NInput
                  placeholder={i18nCookie("domainPlaceholder")}
                  clearable
                  onUpdateValue={(value) => {
                    this.updateValues.domain = value;
                  }}
                />
              )}
            </NFormItem>
          </NGi>
          <NGi span={8}>
            <NFormItem label={i18nCookie("value")} path="value">
              <NInput
                defaultValue={cookie.value}
                placeholder={i18nCookie("valuePlaceholder")}
                clearable
                onUpdateValue={(value) => {
                  this.updateValues.value = value;
                }}
              />
            </NFormItem>
          </NGi>
          <NGi span={16}>
            <NFormItem label={i18nCookie("expires")} path="expires">
              <NDatePicker
                class="widthFull"
                type="datetime"
                shortcuts={shortcuts}
                placeholder={i18nCookie("expiresPlaceholder")}
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
