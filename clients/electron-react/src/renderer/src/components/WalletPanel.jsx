import { useSelector } from 'react-redux'
import WalletControl from './WalletControl';

export default function WalletPanel() {
    const wallets = useSelector(state => state.wallet.wallets);

    let walletList = wallets.map((wallet) => 
        <div>
            <h2>{wallet.name}</h2>
            <WalletControl wallet={wallet} />
        </div>
    );

    return walletList;
}