/**
 * Deluge.UI.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */

/** Dummy translation arrays so Torrent states are available for gettext.js and Translators.
 *
 * All entries in deluge.common.TORRENT_STATE should be added here.
 *
 * No need to import these, just simply use the `_()` function around a status variable.
 */
var TORRENT_STATE_TRANSLATION = [
    _('All'),
    _('Active'),
    _('Allocating'),
    _('Checking'),
    _('Downloading'),
    _('Seeding'),
    _('Paused'),
    _('Checking'),
    _('Queued'),
    _('Error'),
];

/**
 * @static
 * @class Deluge.UI
 * The controller for the whole interface, that ties all the components
 * together and handles the 2 second poll.
 */
deluge.ui = {
    errorCount: 0,

    filters: null,

    /**
     * @description Create all the interface components, the json-rpc client
     * and set up various events that the UI will utilise.
     */
    initialize: function () {
        deluge.add = new Deluge.add.AddWindow();
        deluge.details = new Deluge.details.DetailsPanel();
        deluge.connectionManager = new Deluge.ConnectionManager();
        deluge.editTrackers = new Deluge.EditTrackersWindow();
        deluge.login = new Deluge.LoginWindow();
        deluge.preferences = new Deluge.preferences.PreferencesWindow();
        deluge.sidebar = new Deluge.Sidebar();
        deluge.statusbar = new Deluge.Statusbar();
        deluge.toolbar = new Deluge.Toolbar();

        this.detailsPanel = new Ext.Panel({
            id: 'detailsPanel',
            cls: 'detailsPanel',
            region: 'south',
            split: true,
            height: 215,
            minSize: 100,
            collapsible: true,
            layout: 'fit',
            items: [deluge.details],
        });

        this.MainPanel = new Ext.Panel({
            id: 'mainPanel',
            iconCls: 'x-deluge-main-panel',
            layout: 'border',
            border: false,
            tbar: deluge.toolbar,
            items: [deluge.sidebar, this.detailsPanel, deluge.torrents],
            bbar: deluge.statusbar,
        });

        this.Viewport = new Ext.Viewport({
            layout: 'fit',
            items: [this.MainPanel],
        });

        deluge.events.on('connect', this.onConnect, this);
        deluge.events.on('disconnect', this.onDisconnect, this);
        deluge.client = new Ext.ux.util.RpcClient({
            url: deluge.config.base + 'json',
        });

        // Initialize quicktips so all the tooltip configs start working.
        Ext.QuickTips.init();

        deluge.client.on(
            'connected',
            function (e) {
                deluge.login.show();
            },
            this,
            { single: true }
        );

        this.update = this.update.createDelegate(this);
        this.checkConnection = this.checkConnection.createDelegate(this);

        this.originalTitle = document.title;
    },

    checkConnection: function () {
        deluge.client.web.connected({
            success: this.onConnectionSuccess,
            failure: this.onConnectionError,
            scope: this,
        });
    },

    /**
     * How long between polls, in milliseconds.
     *
     * Read from the server's configuration so it can be turned down on a slow
     * link or a busy daemon, and so it is one number rather than the five
     * copies of `2000` this file used to carry.
     */
    pollInterval: function () {
        var configured = deluge.config ? deluge.config.poll_interval : null;
        return configured > 0 ? configured : 2000;
    },

    /**
     * Schedules the next poll, replacing any already pending.
     */
    schedule: function () {
        if (this.running) {
            clearTimeout(this.running);
        }
        this.running = setTimeout(this.update, this.pollInterval());
    },

    update: function () {
        // Nine places call this directly: every filter click, every menu
        // action, every toolbar button. Without clearing the pending timer
        // each of those fires a poll on top of the scheduled one, so a few
        // clicks in a row produce a burst of overlapping requests that the
        // interface does not need and the daemon has to answer.
        if (this.running) {
            clearTimeout(this.running);
            this.running = undefined;
        }
        if (this.inFlight) {
            // The previous poll has not answered yet, and asking again now
            // would only queue work behind it. Forgetting the request is the
            // part that was wrong: what asks for a refresh is usually a click
            // on a filter, and dropping it left the new filter waiting for the
            // next scheduled poll, so the list changed seconds after the
            // click. Remembered here, sent the moment the current one answers.
            this.wanted = true;
            return;
        }
        this.wanted = false;
        this.inFlight = true;

        var filters = deluge.sidebar.getFilterStates();

        // The search box narrows whatever the sidebar selected, rather than
        // replacing it: searching inside a label is the useful thing to do.
        var search = deluge.toolbar && deluge.toolbar.getSearch
            ? deluge.toolbar.getSearch()
            : '';
        if (search) {
            filters['keyword'] = search;
        }

        this.oldFilters = this.filters;
        this.filters = filters;

        deluge.client.web.update_ui(Deluge.Keys.Grid, filters, {
            success: this.onUpdate,
            failure: this.onUpdateError,
            scope: this,
        });
        deluge.details.update();
    },

    onConnectionError: function (error) {
        if (this.checking) {
            clearTimeout(this.checking);
        }
        this.checking = setTimeout(this.checkConnection, 2000);
    },

    onConnectionSuccess: function (result) {
        if (this.checking) {
            clearTimeout(this.checking);
            this.checking = undefined;
        }
        this.update();
        deluge.statusbar.setStatus({
            iconCls: 'x-deluge-statusbar icon-ok',
            text: _('Connection restored'),
        });
        if (!result) {
            deluge.connectionManager.show();
        }
    },

    onUpdateError: function (error) {
        // Without this the loop would never poll again after one failure,
        // because `update` refuses to start while a poll is in flight.
        this.inFlight = false;
        // A refresh asked for while this poll was failing is not worth
        // chasing: the reconnection below starts a fresh one.
        this.wanted = false;
        if (this.errorCount == 2) {
            Ext.MessageBox.show({
                title: _('Lost Connection'),
                msg: _('The connection to the webserver has been lost!'),
                buttons: Ext.MessageBox.OK,
                icon: Ext.MessageBox.ERROR,
            });
            deluge.events.fire('disconnect');
            deluge.statusbar.setStatus({
                text: _('Lost connection to webserver'),
            });
            this.checking = setTimeout(this.checkConnection, 2000);
        }
        this.errorCount++;
        if (this.running) {
            clearTimeout(this.running);
            this.running = undefined;
        }
    },

    /**
     * @static
     * @private
     * Updates the various components in the interface.
     */
    onUpdate: function (data) {
        this.inFlight = false;
        if (this.running) {
            clearTimeout(this.running);
            this.running = undefined;
        }
        if (!data['connected']) {
            deluge.connectionManager.disconnect(true);
            return;
        }
        this.schedule();

        if (deluge.config.show_session_speed) {
            document.title =
                'D: ' +
                fsize_short(data['stats'].download_rate, true) +
                ' U: ' +
                fsize_short(data['stats'].upload_rate, true) +
                ' - ' +
                this.originalTitle;
        }
        if (Ext.areObjectsEqual(this.filters, this.oldFilters)) {
            deluge.torrents.update(data['torrents']);
        } else {
            deluge.torrents.update(data['torrents'], true);
        }
        deluge.statusbar.update(data['stats']);
        deluge.sidebar.update(data['filters']);
        this.errorCount = 0;

        // Somebody asked for a refresh while this one was in flight. It is
        // answered now rather than at the next tick, which is what makes a
        // filter click feel immediate whatever the poll was doing.
        if (this.wanted) {
            this.update();
        }
    },

    /**
     * @static
     * @private
     * Start the Deluge UI polling the server and update the interface.
     */
    onConnect: function () {
        if (!this.running) {
            this.update();
        }
    },

    /**
     * @static
     * @private
     */
    onDisconnect: function () {
        this.stop();
    },

    /**
     * @static
     * Stop the Deluge UI polling the server and clear the interface.
     */
    stop: function () {
        if (this.running) {
            clearTimeout(this.running);
            this.running = undefined;
            deluge.torrents.getStore().removeAll();
        }
    },
};

Ext.onReady(function (e) {
    deluge.ui.initialize();
});
