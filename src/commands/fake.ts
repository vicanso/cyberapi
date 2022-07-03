import localforage from "localforage";

const stores = new Map<string, LocalForage>();

interface WithID {
  id: string;
}

function getStore(name: string): LocalForage {
  let store = stores.get(name);
  if (store) {
    return store;
  }
  store = localforage.createInstance({
    name,
  });
  stores.set(name, store);
  return store;
}

export async function fakeList<T>(storeName: string): Promise<T[]> {
  const store = getStore(storeName);

  const result = await store.getItem<T[]>("fake");
  if (result != null) {
    return result;
  }
  return [];
}
export async function fakeAdd<T>(storeName: string, data: T) {
  const result = await fakeList<T>(storeName);
  result.push(data);
  const store = getStore(storeName);
  await store.setItem("fake", result);
}

export async function fakeUpdate<T extends WithID>(storeName: string, data: T) {
  const result = await fakeList<T>(storeName);
  let found = -1;
  result.forEach((item, index) => {
    if (item.id == data.id) {
      found = index;
    }
  });
  if (found !== -1) {
    result[found] = data;
  }
  const store = getStore(storeName);
  await store.setItem("fake", result);
}
