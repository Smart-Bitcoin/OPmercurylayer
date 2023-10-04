// Import our outputted wasm ES6 module
// Which, export default's, an initialization function
import init from "./pkg/my_project.js";

const runWasm = async () => {
  // Instantiate our wasm module
  const mercury_wasm = await init("./pkg/my_project_bg.wasm");

  // Call the Add function export from wasm, save the result
  const mnemonic = "praise you muffin lion enable neck grocery crumble super myself license ghost";

  const wallet_name = "MyWallet";

  const wallet = mercury_wasm.fromMnemonic(wallet_name, mnemonic);

  console.log(wallet);

  console.log(mercury_wasm.getSCAddress(wallet, 1));

  const balance = mercury_wasm.getBalance(wallet);

  const address = mercury_wasm.getSCAddress(wallet, 1);

  // Set the result onto the body
  document.body.textContent = `Wallet mnemonic: ${mnemonic} Wallet balance: ${balance} Wallet address: ${address}`;

};
runWasm();
