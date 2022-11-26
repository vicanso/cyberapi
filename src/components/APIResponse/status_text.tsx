import { NGradientText } from "naive-ui";
import { defineComponent } from "vue";
import { getStatusText } from "../../commands/http_response";

function getStatusType(statusCode: number) {
  if (statusCode >= 500) {
    return "error";
  }
  if (statusCode >= 400) {
    return "warning";
  }
  return "success";
}

export default defineComponent({
  name: "APIResponseStatusText",
  props: {
    statusCode: {
      type: Number,
      required: true,
    },
  },
  render() {
    const { statusCode } = this.$props;
    return (
      <NGradientText type={getStatusType(statusCode)}>
        {statusCode} {getStatusText(statusCode)}
      </NGradientText>
    );
  },
});
