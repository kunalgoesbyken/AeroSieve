const fetchOk = async (url) => {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response;
};

export const fetchJson = async (url) => (await fetchOk(url)).json();

export const fetchBytes = async (url) => (await fetchOk(url)).arrayBuffer();
