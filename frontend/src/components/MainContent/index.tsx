'use client';

import React from 'react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => {
  const { isCollapsed } = useSidebar();

  return (
    <main
      className={`flex-1 min-w-0 transition-all duration-300 ${
        isCollapsed ? 'ml-16' : 'ml-16 md:ml-64'
      }`}
    >
      <div className="pl-3 md:pl-8">
        {children}
      </div>
    </main>
  );
};

export default MainContent;
