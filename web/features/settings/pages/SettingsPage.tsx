import React from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import GeneralSettingsPage, {
  isSettingsSection,
  type SettingsSection,
} from './GeneralSettingsPage';

const SettingsPage: React.FC = () => {
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedSection = searchParams.get('section');
  const activeSection: SettingsSection = isSettingsSection(requestedSection)
    ? requestedSection
    : 'general';

  // The gateway settings used to live at `/settings?section=gateway`; keep the
  // old link working so bookmarks and in-app shortcuts do not go stale.
  React.useEffect(() => {
    if (requestedSection === 'gateway') {
      navigate('/gateway/settings', { replace: true });
    }
  }, [navigate, requestedSection]);

  const handleSectionChange = React.useCallback(
    (section: SettingsSection) => {
      const nextSearchParams = new URLSearchParams(searchParams);
      nextSearchParams.set('section', section);
      setSearchParams(nextSearchParams, { replace: true });
    },
    [searchParams, setSearchParams],
  );

  return (
    <GeneralSettingsPage
      activeSection={activeSection}
      onSectionChange={handleSectionChange}
    />
  );
};

export default SettingsPage;
