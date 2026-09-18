/**
 * Deluge.CreateTorrentWindow.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Making a `.torrent` from what is on the daemon's disk.
 *
 * Deluge's Web UI never had this: the GTK client could make a torrent and the
 * browser could not, so a headless install could not either. The toolbar has
 * carried a hidden `Create` button since Deluge, wired to nothing.
 *
 * Two things shape this window. The first is that the files are on the
 * *daemon's* disk, not the browser's, so there is nothing to upload and nothing
 * a file picker could offer: the whole of choosing what to seed is browsing a
 * filesystem this machine cannot see. That is what the first tab is, and it
 * starts at `/` because the content can be anywhere the daemon can read.
 *
 * The second is that hashing takes minutes. The daemon serves one call at a
 * time per connection and this interface has one, so this never calls the
 * method that waits — `redeluge.create_torrent` starts a job and answers at
 * once, and the window asks how it is going until it is done. Closing the
 * window does not stop the job; it is the daemon's, not this dialog's.
 */
Ext.ns('Deluge');

/**
 * The piece sizes worth offering.
 *
 * Powers of two, which is what libtorrent accepts — it throws rather than
 * rounding. `Auto` sends zero and lets it choose from the total size, which is
 * the right answer for nearly everybody and so is the default.
 */
Deluge.PIECE_SIZES = [
    [0, _('Auto')],
    [16384, '16 KiB'],
    [32768, '32 KiB'],
    [65536, '64 KiB'],
    [131072, '128 KiB'],
    [262144, '256 KiB'],
    [524288, '512 KiB'],
    [1048576, '1 MiB'],
    [2097152, '2 MiB'],
    [4194304, '4 MiB'],
    [8388608, '8 MiB'],
    [16777216, '16 MiB'],
];

/**
 * @class Deluge.CreateTorrentWindow
 * @extends Ext.Window
 */
Deluge.CreateTorrentWindow = Ext.extend(Ext.Window, {
    title: _('Create Torrent'),
    width: 640,
    height: 560,
    layout: 'fit',
    buttonAlign: 'right',
    closeAction: 'hide',
    // The whole window, not just its header. A browser shorter than this
    // window put the bottom half of the form — the fieldset, the buttons, the
    // progress bar — below the fold, where nothing could reach it, and the
    // form looked like a column of text boxes running off the screen.
    constrain: true,
    plain: true,
    minWidth: 480,
    // Low enough to fit inside a short browser. The form scrolls inside itself
    // rather than the window growing past the edge.
    minHeight: 320,

    /** What it opens at when there is room for it. */
    preferredWidth: 640,
    preferredHeight: 560,

    /** How often the job is asked how it is going, in milliseconds. */
    pollInterval: 500,

    initComponent: function () {
        // Before the superclass, because a panel reads its bottom bar while it
        // is building itself and one attached afterwards never appears.
        //
        // The width is not decoration. A progress bar in a toolbar has no
        // width of its own and lays out as a strip of nothing: the bar was
        // there, and hashing did move it, and none of that was visible. It is
        // set again on every resize, in `fit`, because a fixed one is wrong at
        // every size but the one it was chosen for.
        this.progress = new Ext.ProgressBar({
            width: 560,
            text: _('Ready'),
        });
        this.bbar = new Ext.Toolbar({ items: [this.progress] });

        Deluge.CreateTorrentWindow.superclass.initComponent.call(this);

        // Enabled by a finished build. Torrents of a few hundred megabytes are
        // built and handed over before a person has looked up, so the window
        // has to still be able to hand it over afterwards — and a browser that
        // blocked the automatic one leaves this as the way to get it at all.
        this.downloadButton = this.addButton(_('Download'), this.download, this);
        this.downloadButton.disable();
        this.createButton = this.addButton(_('Create'), this.onCreate, this);
        this.addButton(_('Close'), this.hide, this);

        this.tabs = this.add({
            xtype: 'tabpanel',
            activeTab: 0,
            border: false,
            deferredRender: false,
            items: [this.buildContentTab(), this.buildOptionsTab()],
            listeners: {
                // A tab that was not the active one while the window was being
                // sized has no width to anchor its fields against, and it does
                // not recalculate on its own: the form came up as a column of
                // boxes a few pixels wide with the rest of the tab empty.
                tabchange: function (panel, tab) {
                    if (tab) tab.doLayout();
                },
            },
        });

        // A hidden window that went on polling would keep asking the daemon
        // about a job nobody is watching.
        this.on('hide', this.stopPolling, this);
        this.on('destroy', this.stopPolling, this);

        // The browser can be resized while this is open, and a window that
        // fitted when it opened does not fit afterwards.
        Ext.EventManager.onWindowResize(this.fit, this);
    },

    // ------------------------------------------------------------- the disk

    buildContentTab: function () {
        this.entries = new Ext.data.JsonStore({
            fields: ['name', 'path', 'kind', 'size'],
            root: 'entries',
        });

        this.pathField = new Ext.form.TextField({
            emptyText: _('Path on the daemon'),
            width: 380,
            enableKeyEvents: true,
            listeners: {
                specialkey: {
                    fn: function (field, event) {
                        if (event.getKey() === event.ENTER) {
                            this.browse(field.getValue());
                        }
                    },
                    scope: this,
                },
            },
        });

        this.upButton = new Ext.Button({
            text: _('Up'),
            iconCls: 'icon-up',
            handler: this.onUp,
            scope: this,
        });

        this.listing = new Ext.grid.GridPanel({
            store: this.entries,
            border: false,
            autoExpandColumn: 'name',
            // One row at a time: a torrent is made from one path, and a grid
            // that lets four be highlighted only invites the question of what
            // it would do with them.
            selModel: new Ext.grid.RowSelectionModel({ singleSelect: true }),
            columns: [
                {
                    id: 'name',
                    header: _('Name'),
                    dataIndex: 'name',
                    sortable: false,
                    renderer: function (value, meta, record) {
                        var icon =
                            record.get('kind') === 'dir'
                                ? 'x-deluge-browse-dir'
                                : 'x-deluge-browse-file';
                        return String.format(
                            '<div class="x-deluge-browse-row {0}">{1}</div>',
                            icon,
                            Ext.util.Format.htmlEncode(value)
                        );
                    },
                },
                {
                    header: _('Size'),
                    dataIndex: 'size',
                    width: 90,
                    align: 'right',
                    sortable: false,
                    renderer: function (value, meta, record) {
                        // A directory's size means walking all of it, which is
                        // a question asked of the one that was chosen rather
                        // than of every row on the way past.
                        if (record.get('kind') === 'dir') return '';
                        return fsize(value);
                    },
                },
            ],
            listeners: {
                rowclick: { fn: this.onRowClick, scope: this },
                rowdblclick: { fn: this.onRowDoubleClick, scope: this },
            },
        });

        // What has been chosen, where a person looks last before pressing the
        // button. A toolbar item rather than a field: it is read, never typed
        // into, and a disabled text box invites clicking at it.
        this.chosen = new Ext.Toolbar.TextItem({
            text: _('Nothing chosen yet'),
        });

        return {
            title: _('Content'),
            layout: 'fit',
            tbar: [
                this.upButton,
                ' ',
                this.pathField,
                ' ',
                {
                    text: _('Go'),
                    handler: function () {
                        this.browse(this.pathField.getValue());
                    },
                    scope: this,
                },
            ],
            items: this.listing,
            bbar: [this.chosen],
        };
    },

    /**
     * Lists one directory and shows it.
     *
     * Asking about a file lists the directory it is in, which the daemon does
     * rather than this: clicking a file to choose it must not empty the list
     * being chosen from.
     */
    browse: function (path) {
        deluge.client.redeluge.list_directory(path || '/', {
            success: function (listing) {
                this.here = listing.path;
                this.pathField.setValue(listing.path);
                this.upButton.setDisabled(!listing.parent);
                this.parentPath = listing.parent;
                this.entries.loadData(listing);
            },
            failure: function () {
                Ext.MessageBox.show({
                    title: _('Create Torrent'),
                    msg: String.format(
                        _('Could not read {0}'),
                        Ext.util.Format.htmlEncode(path)
                    ),
                    buttons: Ext.MessageBox.OK,
                    icon: Ext.MessageBox.WARNING,
                });
            },
            scope: this,
        });
    },

    onUp: function () {
        if (this.parentPath) this.browse(this.parentPath);
    },

    onRowClick: function (grid, index) {
        this.choose(this.entries.getAt(index));
    },

    onRowDoubleClick: function (grid, index) {
        var record = this.entries.getAt(index);
        // A directory opens; a file has nowhere to open to, so the second
        // click means the same as the first.
        if (record.get('kind') === 'dir') {
            this.browse(record.get('path'));
            // Still chosen: opening a folder to look inside it is the usual
            // way of deciding to seed that folder, and having to click it once
            // more on the way past would be a trap.
            this.choose(record);
        }
    },

    /** Takes one entry as what the torrent will be made from. */
    choose: function (record) {
        if (!record) return;
        this.selected = record.get('path');
        this.chosen.setText(
            String.format(
                _('Chosen: {0}'),
                Ext.util.Format.htmlEncode(this.selected)
            )
        );

        // What it comes to on disk, which is the one number that says whether
        // this is the five minute job or the five hour one. Asked of the
        // daemon, because only it can see the files.
        deluge.client.core.get_path_size(this.selected, {
            success: function (size) {
                if (this.selected !== record.get('path')) return;
                if (size === null || size < 0) return;
                this.chosen.setText(
                    String.format(
                        _('Chosen: {0} ({1})'),
                        Ext.util.Format.htmlEncode(this.selected),
                        fsize(size)
                    )
                );
            },
            scope: this,
        });
    },

    // ---------------------------------------------------------- the options

    buildOptionsTab: function () {
        this.form = new Ext.form.FormPanel({
            title: _('Options'),
            border: false,
            autoScroll: true,
            bodyStyle: 'padding: 10px;',
            labelWidth: 130,
            defaults: { anchor: '96%' },
            items: [
                {
                    xtype: 'textarea',
                    name: 'trackers',
                    fieldLabel: _('Trackers'),
                    height: 70,
                    // One per line, and a blank line is dropped rather than
                    // becoming an announce URL every client then fails on.
                    emptyText: _('One announce URL per line'),
                },
                {
                    xtype: 'textarea',
                    name: 'webseeds',
                    fieldLabel: _('Web seeds'),
                    height: 50,
                    emptyText: _('One URL per line'),
                },
                {
                    xtype: 'textfield',
                    name: 'comment',
                    fieldLabel: _('Comment'),
                },
                {
                    xtype: 'combo',
                    name: 'piece_length',
                    fieldLabel: _('Piece size'),
                    store: new Ext.data.ArrayStore({
                        fields: ['value', 'label'],
                        data: Deluge.PIECE_SIZES,
                    }),
                    valueField: 'value',
                    displayField: 'label',
                    mode: 'local',
                    editable: false,
                    triggerAction: 'all',
                    value: 0,
                },
                {
                    xtype: 'combo',
                    name: 'torrent_format',
                    fieldLabel: _('Format'),
                    store: new Ext.data.ArrayStore({
                        fields: ['value', 'label'],
                        data: [
                            ['v1', _('v1 (every client)')],
                            ['hybrid', _('Hybrid v1 + v2')],
                            ['v2', _('v2 only')],
                        ],
                    }),
                    valueField: 'value',
                    displayField: 'label',
                    mode: 'local',
                    editable: false,
                    triggerAction: 'all',
                    // A hybrid torrent is refused by trackers that only know
                    // v1, and there is no way to tell from here which those
                    // are. The one that works everywhere is the default.
                    value: 'v1',
                },
                {
                    xtype: 'checkbox',
                    name: 'private',
                    fieldLabel: '',
                    labelSeparator: '',
                    boxLabel: _('Private (no DHT, no peer exchange)'),
                },
                {
                    xtype: 'fieldset',
                    title: _('When it is built'),
                    autoHeight: true,
                    // Its own, narrower than the form's: a fieldset is indented
                    // and inherits a label column it does not have room for, so
                    // `Save on the daemon` wrapped onto two lines.
                    labelWidth: 110,
                    defaults: { anchor: '96%' },
                    style: 'margin-top: 10px;',
                    items: [
                        {
                            xtype: 'checkbox',
                            name: 'download',
                            fieldLabel: '',
                            labelSeparator: '',
                            boxLabel: _('Send it to this browser'),
                            checked: true,
                        },
                        {
                            // On by default. A torrent nobody is seeding is a
                            // file nobody can fetch: whoever it is sent to sits
                            // at nought per cent until the person who made it
                            // works out that making it was not the last step.
                            xtype: 'checkbox',
                            name: 'add_to_session',
                            fieldLabel: '',
                            labelSeparator: '',
                            boxLabel: _('Add it and start seeding'),
                            checked: true,
                        },
                        {
                            xtype: 'textfield',
                            name: 'target',
                            fieldLabel: _('Write it to'),
                            emptyText: _('Optional: a path to write it to'),
                        },
                    ],
                },
            ],
        });
        return this.form;
    },

    /** The form as the daemon's argument dictionary. */
    options: function () {
        var values = this.form.getForm().getFieldValues();
        var lines = function (text) {
            if (!text) return [];
            return text
                .split('\n')
                .map(function (line) {
                    return line.trim();
                })
                .filter(function (line) {
                    return line.length > 0;
                });
        };

        return {
            path: this.selected,
            trackers: lines(values.trackers),
            webseeds: lines(values.webseeds),
            comment: values.comment || '',
            piece_length: parseInt(values.piece_length, 10) || 0,
            torrent_format: values.torrent_format || 'v1',
            private: !!values.private,
            add_to_session: !!values.add_to_session,
            target: (values.target || '').trim(),
        };
    },

    // ------------------------------------------------------------ the build

    onCreate: function () {
        if (!this.selected) {
            Ext.MessageBox.show({
                title: _('Create Torrent'),
                msg: _('Choose a file or a folder on the Content tab first.'),
                buttons: Ext.MessageBox.OK,
                icon: Ext.MessageBox.INFO,
            });
            this.tabs.setActiveTab(0);
            return;
        }

        var values = this.form.getForm().getFieldValues();
        this.wantsDownload = !!values.download;

        this.createButton.disable();
        this.downloadButton.disable();
        this.progress.updateProgress(0, _('Starting'));

        deluge.client.redeluge.create_torrent(this.options(), {
            success: function (job) {
                this.job = job;
                this.poll();
            },
            failure: function (error) {
                this.finish(false, error);
            },
            scope: this,
        });
    },

    poll: function () {
        this.stopPolling();
        this.timer = setTimeout(this.ask.createDelegate(this), this.pollInterval);
    },

    stopPolling: function () {
        if (this.timer) {
            clearTimeout(this.timer);
            this.timer = null;
        }
    },

    ask: function () {
        if (!this.job) return;
        deluge.client.redeluge.get_create_torrent(this.job, {
            success: function (status) {
                if (status.state === 'running') {
                    this.progress.updateProgress(
                        status.progress,
                        String.format(
                            _('Hashing {0}% — piece {1} of {2}'),
                            Math.round(status.progress * 100),
                            status.piece,
                            status.pieces
                        )
                    );
                    this.poll();
                    return;
                }
                this.finish(status.state === 'done', status.error, status);
            },
            // A failed poll is not a failed build: the answer may simply not
            // have arrived. Asking again is cheaper than telling somebody
            // their torrent broke when it did not.
            failure: function () {
                this.poll();
            },
            scope: this,
        });
    },

    finish: function (worked, error, status) {
        this.stopPolling();
        this.createButton.enable();

        if (!worked) {
            this.progress.updateProgress(0, _('Failed'));
            Ext.MessageBox.show({
                title: _('Create Torrent'),
                msg: Ext.util.Format.htmlEncode(
                    error || _('The torrent could not be created.')
                ),
                buttons: Ext.MessageBox.OK,
                icon: Ext.MessageBox.ERROR,
            });
            return;
        }

        // Everything that happened, on the one line a person is already
        // looking at. This used to say nothing at all unless the torrent had
        // been written somewhere or added, so the ordinary case — build it and
        // send it to the browser — finished in silence.
        // Kept short, because it has to fit on a bar: a full sentence per
        // outcome ran off the end of it and the last one was the one that said
        // the torrent had been added.
        var done = [String.format(_('Created {0}'), (status && status.name) || '')];
        if (status && status.size) done.push(fsize(status.size));
        if (status && status.written_to) done.push(_('saved'));
        if (status && status.torrent_id) done.push(_('seeding'));

        this.progress.updateProgress(1, done.join(' · '));
        // Where it went, which is too long for the bar and is worth keeping in
        // front of somebody who asked for it.
        if (status && status.written_to) {
            this.chosen.setText(
                String.format(
                    _('Written to {0}'),
                    Ext.util.Format.htmlEncode(status.written_to)
                )
            );
        }
        this.downloadButton.enable();
        if (this.wantsDownload) this.download();
    },

    /**
     * Hands the file to the browser.
     *
     * Through a hidden iframe rather than by navigating: the response is an
     * attachment, and pointing the page itself at it works in most browsers and
     * throws away the interface in the ones where it does not.
     */
    download: function () {
        // Also a button handler, and a button can be clicked in the moment
        // between a window opening and a torrent existing.
        if (!this.job) return;
        var url = deluge.config.base + 'created/' + encodeURIComponent(this.job);
        if (!this.sink) {
            this.sink = Ext.DomHelper.append(
                document.body,
                { tag: 'iframe', style: 'display: none;' },
                true
            );
        }
        this.sink.dom.src = url;
    },

    /**
     * Sizes the window to the browser, which may be smaller than it is.
     *
     * Without this the window opens at its preferred size whatever the room,
     * and everything past the bottom edge is simply unreachable: no scrollbar
     * appears, because it is the window that overflows and not the page.
     */
    fit: function () {
        if (!this.rendered || !this.isVisible()) return;
        var view = Ext.getBody().getViewSize();
        this.setSize(
            Math.min(this.preferredWidth, view.width - 20),
            Math.min(this.preferredHeight, view.height - 20)
        );
        this.center();
        // The bar spans whatever width the window ended up with, less the
        // toolbar's own padding.
        this.progress.setWidth(this.getWidth() - 34);
        // Every tab, not just the one on top: the anchored fields of the
        // others are sized from a width they only have once this has run.
        this.doLayout(false, true);
    },

    show: function () {
        Deluge.CreateTorrentWindow.superclass.show.call(this);
        this.fit();

        // Every time, not once: the daemon may have been swapped for another
        // one with an entirely different disk since this window was last open.
        this.selected = null;
        this.job = null;
        this.chosen.setText(_('Nothing chosen yet'));
        this.progress.updateProgress(0, _('Ready'));
        this.downloadButton.disable();
        this.createButton.enable();

        // Start where the downloads are, because that is where the thing being
        // shared almost always is. It is a core setting rather than one this
        // page was given, so it has to be asked for; `/` is the fallback, and
        // the browser can climb anywhere from either.
        deluge.client.core.get_config_value('download_location', {
            success: function (location) {
                this.browse(location || '/');
            },
            failure: function () {
                this.browse('/');
            },
            scope: this,
        });
    },
});
