import axios, { AxiosResponse } from "axios";

const request = axios.create({
  // 默认超时为10秒
  timeout: 10 * 1000,
});

request.interceptors.request.use(
  (config) => {
    config.url = `/api${config.url}`;

    return config;
  },
  (err) => {
    return Promise.reject(err);
  }
);

request.interceptors.response.use((resp: AxiosResponse) => {
  if (resp.status >= 400) {
    return Promise.reject(resp.data.message);
  }
  return Promise.resolve(resp);
});

export default request;
