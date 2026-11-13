# Share one popup shell

The Model menu and settings choice menus need the same open, close, outside-click, Escape, and focus rules. The window now uses one popup shell for those rules and keeps menu-specific rows and positioning in each caller. Keeping two popup implementations would make every popup fix land twice and let the two menus drift.
