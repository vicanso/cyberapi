import { storeToRefs } from "pinia";
import { defineComponent, onBeforeMount, ref } from "vue";
import {
  NCard,
  useMessage,
  DataTableColumns,
  NDataTable,
  NButton,
  NIcon,
  NSpace,
  useDialog,
} from "naive-ui";
import { css } from "@linaria/core";

import { useCookieStore } from "../stores/cookie";
import { showError, getNormalDialogStyle } from "../helpers/util";
import { i18nCookie, i18nCommon } from "../i18n";
import { CreateOutline, TrashOutline } from "@vicons/ionicons5";
import { Cookie } from "../commands/cookies";
import ExCookieEditor from "../components/ExCookieEditor";
import { margin } from "../constants/style";

enum Mode {
  Edit = "edit",
  List = "list",
}

const addBtnClass = css`
  margin-top: ${margin}px;
  width: 100%;
`;

export default defineComponent({
  name: "CookieSetting",
  setup() {
    const message = useMessage();
    const dialog = useDialog();
    const cookieStore = useCookieStore();
    const { cookies } = storeToRefs(cookieStore);
    const mode = ref(Mode.List);
    const updatedCookie = ref({} as Cookie);
    onBeforeMount(async () => {
      try {
        await cookieStore.fetch();
      } catch (err) {
        showError(message, err);
      }
    });
    const removeCookie = async (index: number) => {
      const cookie = cookieStore.cookies[index];
      const d = dialog.warning({
        title: i18nCookie("deleteCookie"),
        content: i18nCookie("deleteCookieContent"),
        positiveText: i18nCommon("confirm"),
        onPositiveClick: async () => {
          d.loading = true;
          try {
            await cookieStore.remove(cookie);
          } catch (err) {
            showError(message, err);
          } finally {
            d.loading = false;
          }
        },
      });
    };
    const editCookie = (index: number) => {
      const cookie = cookieStore.cookies[index];
      updatedCookie.value = cookie;
      mode.value = Mode.Edit;
    };
    const addCookie = () => {
      updatedCookie.value = {} as Cookie;
      mode.value = Mode.Edit;
    };

    return {
      mode,
      cookies,
      removeCookie,
      editCookie,
      addCookie,
      updatedCookie,
    };
  },
  render() {
    const { cookies, removeCookie, editCookie, mode, updatedCookie } = this;
    const modalStyle = getNormalDialogStyle(0.8);
    let cookieNameMaxWidth = 0;
    let domainNameMaxWidth = 0;
    const fontWidth = 7;
    cookies.forEach((item) => {
      const cookieWidth = item.name.length * fontWidth;
      const domainWidth = item.domain.length * fontWidth;
      if (cookieWidth > cookieNameMaxWidth) {
        cookieNameMaxWidth = cookieWidth;
      }
      if (domainWidth > domainNameMaxWidth) {
        domainNameMaxWidth = domainWidth;
      }
    });
    const tablePadding = 10;

    const columns: DataTableColumns = [
      {
        title: i18nCookie("name"),
        key: "name",
        minWidth: cookieNameMaxWidth + tablePadding,
        ellipsis: {
          tooltip: true,
        },
      },
      {
        title: i18nCookie("value"),
        key: "value",
      },
      {
        title: i18nCookie("path"),
        key: "path",
        width: 60,
      },
      {
        title: i18nCookie("domain"),
        key: "domain",
        width: domainNameMaxWidth + tablePadding,
      },
      {
        title: i18nCookie("expires"),
        key: "expires",
        width: 200,
      },
      {
        title: i18nCookie("op"),
        key: "op",
        width: 120,
        render: (row, index) => {
          return (
            <NSpace>
              <NButton
                onClick={() => {
                  editCookie(index);
                }}
              >
                <NIcon>
                  <CreateOutline />
                </NIcon>
              </NButton>
              <NButton
                onClick={() => {
                  removeCookie(index);
                }}
              >
                <NIcon>
                  <TrashOutline />
                </NIcon>
              </NButton>
            </NSpace>
          );
        },
      },
    ];

    return (
      <NCard title={i18nCookie("title")} style={modalStyle}>
        {mode === Mode.List && (
          <div>
            <NDataTable data={cookies} columns={columns} />
            <NButton
              class={addBtnClass}
              onClick={() => {
                this.addCookie();
              }}
            >
              {i18nCommon("add")}
            </NButton>
          </div>
        )}
        {mode === Mode.Edit && (
          <ExCookieEditor
            cookie={updatedCookie}
            onBack={() => {
              this.mode = Mode.List;
            }}
          />
        )}
      </NCard>
    );
  },
});
