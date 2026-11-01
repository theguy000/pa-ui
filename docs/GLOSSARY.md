# Prime Agent

The desktop window onto a running `prime-agent` daemon: a session's transcript, the composer, the queue and the model menu.

## Language

### Conversation

**Session**:
A conversation the daemon keeps. Its id can be re-keyed when the daemon restarts; the session is still the same session.
_Avoid_: chat, thread, conversation

**Turn**:
One prompt and the agent's response to it. A session holds many turns, one at a time.
_Avoid_: run, request, exchange

**Message**:
One entry in a session's store: a prompt, an assistant reply, or a tool result. A message is not a row; one message can occupy several rows.
_Avoid_: entry, record, item

**Store**:
The daemon's record of a session's messages, and the authority on what happened there. The window reconciles toward it rather than trusting the live stream.
_Avoid_: cache, database, history, backend

### What the reader sees

**Sidebar**:
The column of sessions down the window's left: where the reader picks a session or starts one. It folds to a strip of controls and unfolds again.
_Avoid_: rail, side panel, nav

**Composer**:
The strip along the window's foot where the reader writes: the draft, whatever the draft is carrying, and the control that sends it — or stops the turn instead. It is never a message; a message is what the composer produces.
_Avoid_: input box, text area, prompt box, message box

**Reader**:
The person looking at the window. They own the scroll position and whatever they have opened.
_Avoid_: user (that is the role a message carries, or the person the agent serves)

**Transcript**:
A session's rows, in order, as the reader reads them.
_Avoid_: chat history, log, content

**Sheet**:
The rows of a transcript as the reader sees them: what is scrolled, paged, opened and closed. Named for a sheet of paper covered in rows, not for the thing that draws it.
_Avoid_: list, model, view, buffer, pane

**Row**:
One drawn block of the transcript: prose, a code block, a table, a tool cell, an image. Rows are what the reader opens and closes.
_Avoid_: entry, line, block, item

**Cell**:
The row that stands for one tool call: what ran, what it printed, and whether it failed.
_Avoid_: tool row, tool result (the result is a message; the cell is the row)

**Live turn**:
The turn still being written. Its rows are provisional until the store carries the message they belong to.
_Avoid_: streaming message, open message, partial row

### Colour

**Theme**:
The colours the window draws with: the set compiled into the window, or that set with a reader's file laid over it. A theme sets base values and, where it has to, single roles.
_Avoid_: skin, scheme, look

**Palette**:
The base values a theme sets: the window's background, its ink and quiet ink, danger. Every role is made of one of these unless the theme names that role itself.
_Avoid_: colours, swatches

**Role**:
One named colour the window draws: a part, in one state. A component asks for a role and never for a value, so the theme is the only place that decides what a hovered button looks like.
_Avoid_: token, variable, colour constant

### The title bar

**Title bar**:
The strip along the top of the window: the right column's collapse button and the three caption buttons. The window draws it because the native caption cannot follow the theme, and it paints no field of its own — the window's background and the sidebar's header band run under it.
_Avoid_: chrome, header, titlebar, caption (a caption is the text a bar carries)

**Caption button**:
One of the minimise, maximise and close controls at the title bar's right edge. Windows owns the maximise click for Snap Layouts and draws none of the three; the window does, so a theme reaches them.
_Avoid_: window control, chrome button

**Collapse button**:
The control that collapses a column to its strip of controls and back: the sidebar's, at the right of its header, and the right column's, in the title bar. A column has one; the standalone control beside it is the way back open.
_Avoid_: fold, toggle, panel button, expand button

### Paging

**Page**:
A window of a session's messages read from the store in one go. The transcript is walked page by page, newest first.
_Avoid_: chunk, batch, slice

**Page cursor**:
The fingerprint of a page's oldest and newest message, which is how the window tells one page from another when the rows look the same.
_Avoid_: offset, marker

### The queue

**Lane**:
One of the two routes a waiting message takes: steering, delivered inside the turn that is running, or follow-up, delivered after it.
_Avoid_: channel, queue (the queue is both lanes together)

**Parked message**:
A message that waits in a lane instead of running now.
_Avoid_: pending message, queued message
