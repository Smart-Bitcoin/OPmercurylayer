import { useCallback } from "react";
import { useNavigate } from "react-router-dom";
import NavBar from "../components/NavBar";
import WithdrawBTCPanel from "../components/WithdrawBTCPanel";
import WithdrawStatecoinsInfoPanel from "../components/WithdrawStatecoinsInfoPanel";

const WithdrawPage = () => {
  const navigate = useNavigate();

  const onHelpButtonContainerClick = useCallback(() => {
    navigate("/helpandsupportpage");
  }, [navigate]);

  const onCogIconClick = useCallback(() => {
    navigate("/settingspage");
  }, [navigate]);

  const onLogoutButtonIconClick = useCallback(() => {
    navigate("/");
  }, [navigate]);

  return (
    <div className="w-full relative bg-whitesmoke h-[926px] flex flex-col items-center justify-start gap-[25px]">
      <NavBar
        onHelpButtonContainerClick={onHelpButtonContainerClick}
        onCogIconClick={onCogIconClick}
        onLogoutButtonIconClick={onLogoutButtonIconClick}
        showLogoutButton
        showSettingsButton
        showHelpButton
      />
      <div className="self-stretch h-[125px] flex flex-col items-start justify-start py-0 px-5 box-border">
        <WithdrawBTCPanel />
      </div>
      <div className="self-stretch flex-1 overflow-hidden flex flex-row items-center justify-start p-5">
        <WithdrawStatecoinsInfoPanel />
      </div>
    </div>
  );
};

export default WithdrawPage;
