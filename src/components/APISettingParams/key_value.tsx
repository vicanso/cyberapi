import { defineComponent, PropType } from "vue";

import { HTTPRequestKVParam } from "../../commands/http_request";

export default defineComponent({
  name: "APISettingParamsKeyValue",
  props: {
    params: {
      type: Array as PropType<HTTPRequestKVParam[]>,
      required: true,
    },
  },
  setup(props) {
    console.dir(props.params);
  },
  render() {
    return <div>abc</div>;
  },
});
