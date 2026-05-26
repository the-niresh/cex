import type { CURRENCY_TYPE, MARKET_ASSETS } from "../utils/types";

export default class Balance {
  private balance: Record<
    string,
    Partial<
      Record<
        CURRENCY_TYPE,
        {
          total: number;
        }
      >
    >
  >;
  constructor() {
    this.balance = {};
  }

  getOrCreateUserId(userId: string) {
    let userPresent = this.balance[userId];
    if (!userPresent) {
      userPresent = { USD: { total: 100000 }, BTC: { total: 10 } };
      this.balance[userId] = userPresent;
    }
    return userPresent;
  }

  getOrCreateUserCurrency(userId: string, currency: CURRENCY_TYPE) {
    let userPresent = this.getOrCreateUserId(userId);
    if (!userPresent[currency]) {
      userPresent[currency] = { total: 0 };
    }
    return userPresent[currency];
  }

  getUSDBalance(userId: string, currency: CURRENCY_TYPE = "USD") {
    const userCurrency = this.getOrCreateUserCurrency(userId, currency);
    return userCurrency.total;
  }

  addAssetBalance(userId: string, amount: number, currency: CURRENCY_TYPE) {
    const currencyDetails = this.getOrCreateUserCurrency(userId, currency);
    currencyDetails.total += amount;
    return currencyDetails.total;
  }

  deductAssetBalance(userId: string, amount: number, currencyType: CURRENCY_TYPE) {
    const userPresent = this.getOrCreateUserId(userId);
    const currencyDetails = this.getOrCreateUserCurrency(userId, currencyType);
    currencyDetails.total -= amount;
    if (currencyDetails.total === 0) {
      delete userPresent[currencyType];
    }
  }

  getAssetBalance(userId: string, asset: MARKET_ASSETS) {
    const currencyDetails = this.getOrCreateUserCurrency(userId, asset);
    return currencyDetails.total;
  }

  updateAssetQty(userId: string, currencyType: CURRENCY_TYPE, updateBalance: number) {
    let user = this.balance[userId]!;
    const userCurrency = user[currencyType]!;
    userCurrency.total = updateBalance;
  }

  getAllAssets(userId: string) {
    const presentUser = this.getOrCreateUserId(userId);
    let allAssets: [CURRENCY_TYPE, number][] = [];

    Object.keys(presentUser).forEach((currency) => {
      const curr = currency as CURRENCY_TYPE;
      allAssets.push([curr, presentUser[curr]?.total!]);
    });
    return allAssets;
  }

  deleteAssetEntry(userId: string, currencyType: CURRENCY_TYPE) {
    const user = this.balance[userId]!;
    delete user[currencyType];
  }
}
