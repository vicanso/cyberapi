import { NDropdown, NIcon, NSpace, useMessage } from "naive-ui";
import { css } from "@linaria/core";
import prettyBytes from "pretty-bytes";
import { isNumber } from "lodash-es";

import { defineComponent, ref } from "vue";
import {
  getLatestResponseList,
  Response,
  selectResponse,
} from "../../commands/http_response";
import { formatSimpleDate, showError } from "../../helpers/util";
import { ListOutline } from "@vicons/ionicons5";

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
    return {
      responseList,
      handleFetch,
      handleSelect,
    };
  },
  render() {
    const { responseList } = this;
    const options = responseList.map((item, index) => {
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
