export class SaveStore {
  static async open() {
    const db = await new Promise((resolve, reject) => {
      const request = indexedDB.open("zigba-local", 2);
      request.onupgradeneeded = () => {
        for (const name of ["saves", "games", "assets", "scenarios", "reports"])
          if (!request.result.objectStoreNames.contains(name))
            request.result.createObjectStore(name);
      };
      request.onsuccess = () => {
        request.result.onversionchange = () => request.result.close();
        resolve(request.result);
      };
      request.onerror = () => reject(request.error);
      request.onblocked = () =>
        reject(new Error("Close other ZiGBA tabs to open save storage"));
    });
    return new SaveStore(db);
  }
  constructor(db) {
    this.db = db;
  }
  async get(hash) {
    return new Promise((resolve, reject) => {
      const request = this.db
        .transaction("saves")
        .objectStore("saves")
        .get(hash);
      request.onsuccess = () => {
        request.result.onversionchange = () => request.result.close();
        resolve(request.result);
      };
      request.onerror = () => reject(request.error);
    });
  }
  async put(hash, data) {
    const previous = await this.get(hash);
    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("saves", "readwrite");
      transaction
        .objectStore("saves")
        .put({ data, previous: previous?.data, updatedAt: Date.now() }, hash);
      transaction.oncomplete = resolve;
      transaction.onerror = () => reject(transaction.error);
      transaction.onabort = () =>
        reject(transaction.error ?? new Error("Save transaction aborted"));
    });
  }
}

/** Keeps a browser-managed single-writer lock until the returned release call. */
export async function acquireSaveLock(hash) {
  if (!navigator.locks) return null;
  return new Promise((resolve, reject) => {
    navigator.locks
      .request(`zigba-save-${hash}`, { ifAvailable: true }, (lock) => {
        if (!lock) {
          resolve(null);
          return;
        }
        return new Promise((release) => resolve(release));
      })
      .catch(reject);
  });
}
