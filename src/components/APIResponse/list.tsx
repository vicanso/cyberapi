import {
  NButton,
  NDivider,
  NDropdown,
  NIcon,
  NP,
  NSpace,
  useMessage,
} from "naive-ui";
import { css } from "@linaria/core";
import prettyBytes from "pretty-bytes";
import { isNumber } from "lodash-es";

import { defineComponent, ref } from "vue";
import {
  clearLatestResponseList,
  getLatestResponseList,
  Response,
  selectResponse,
} from "../../commands/http_response";
import { formatSimpleDate, showError } from "../../helpers/util";
import {
  ListOutline,
  TrashOutline,
} from "@vicons/ionicons5";
import { DropdownMixedOption } from "naive-ui/es/dropdown/src/interface";
import { i18nStore } from "../../i18n";

const showMoreClass = css`
  cursor: pointer;
  margin: 15px 10px 0 0;
  padding: 0 5px;
`;

export default defineComponent({
  name: "APIResponseList",
  props: {
    id: {
      type: String,
      required: true,
    },
  },
  setup(props) {
    const message = useMessage();
    const responseList = ref([] as Response[]);

    const handleFetch = async () => {
      responseList.value.length = 0;
      try {
        const arr = await getLatestResponseList(props.id);
        responseList.value = arr;
      } catch (err) {
        showError(message, err);
      }
    };
    const handleSelect = (index: number) => {
      const resp = responseList.value[index];
      if (resp) {
        selectResponse(resp.resp);
      }
    };
    const handleClearHistory = async () => {
      try {
        await clearLatestResponseList(props.id);
        message.info(i18nStore("clearHistorySuccess"));
        responseList.value.length = 0;
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      responseList,
      handleFetch,
      handleSelect,
      handleClearHistory,
    };
  },
  render() {
    const { responseList } = this;
    const options: DropdownMixedOption[] = responseList.map((item, index) => {
      let bodySize = "--";
      if (item.resp && isNumber(item.resp.bodySize)) {
        bodySize = prettyBytes(item.resp.bodySize);
      }
      return {
        label: () => (
          <NSpace>
            {item.resp.status}
            {bodySize}
            {formatSimpleDate(item.createdAt)}
          </NSpace>
        ),
        key: index,
      };
    });
    const clearHistorySlots = {
      icon: () => (
        <NIcon>
          <TrashOutline />
        </NIcon>
      ),
    };

    const clearBtn = responseList?.length !== 0 && (
      <NButton
        class="widthFull"
        v-slots={clearHistorySlots}
        quaternary
        onClick={() => {
          this.handleClearHistory();
        }}
      >
        {i18nStore("clearHistory")}
      </NButton>
    );

    const tips = responseList?.length === 0 && (
      <NP
        style={{
          margin: "5px 15px",
        }}
      >
        {i18nStore("noHistory")}
      </NP>
    );
    options.unshift({
      key: "header",
      type: "render",
      render: () => (
        <div>
          {clearBtn}
          <NDivider
            titlePlacement="left"
            style={{
              margin: "5px 0",
              "font-size": "12px",
            }}
          >
            {i18nStore("responseList")}
          </NDivider>
          {tips}
        </div>
      ),
    });
    return (
      <NDropdown
        trigger="click"
        placement="bottom-end"
        onSelect={(value) => {
          this.handleSelect(value);
        }}
        onUpdateShow={(value) => {
          if (value) {
            this.handleFetch();
          }
        }}
        options={options}
        showArrow={true}
      >
        <NIcon class={showMoreClass} size={20}>
          <ListOutline />
        </NIcon>
      </NDropdown>
    );
  },
});
