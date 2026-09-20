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
     * How long a hidden tab waits between the calls that keep it logged in.
     *
     * Well under the shortest session timeout anybody sets, and far longer
     * than anything that would show up as load.
     */
    heartbeatEvery: 15 * 60 * 1000,

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
        this.watchVisibility();

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
     *
     * Nothing is scheduled while the tab is not being looked at. A tab left
     * open in the background polled every two seconds for as long as the
     * browser was running — a walk of the library on the daemon, an answer
     * built and compressed, and a list nobody could see. `onShown` below asks
     * again the moment it comes back, so what is saved is only the work whose
     * result nobody would have seen.
     */
    schedule: function () {
        if (this.running) {
            clearTimeout(this.running);
            this.running = undefined;
        }
        if (this.hidden()) return;
        this.running = setTimeout(this.update, this.pollInterval());
    },

    /**
     * Whether the tab is out of sight.
     *
     * A window behind another window is still visible as far as this is
     * concerned, which is right: a list on a second screen is being looked at.
     * A browser too old to have the property is treated as visible, so it
     * keeps the behaviour it had.
     */
    hidden: function () {
        return typeof document !== 'undefined' && document.hidden === true;
    },

    /**
     * Keeps a hidden tab logged in, and nothing more.
     *
     * The session has a sliding expiry that every call extends, so a tab that
     * polls never expires and one that stops polling does — after an hour, by
     * default. Coming back to a login window because the tab was in the
     * background is not an improvement, so while it is hidden one call goes
     * out every quarter of an hour. It checks the session and touches nothing
     * else: no walk of the library, no list built, about a millionth of what
     * polling would have cost over the same time.
     */
    keepSessionAlive: function () {
        if (this.heartbeat) {
            clearTimeout(this.heartbeat);
        }
        this.heartbeat = setTimeout(
            function () {
                this.heartbeat = undefined;
                if (!this.hidden()) return;
                deluge.client.auth.check_session({
                    success: this.keepSessionAlive,
                    failure: this.keepSessionAlive,
                    scope: this,
                });
            }.createDelegate(this),
            this.heartbeatEvery
        );
    },

    /**
     * Starts and stops the loop as the tab is hidden and shown.
     */
    watchVisibility: function () {
        if (typeof document === 'undefined' || typeof document.hidden !== 'boolean') {
            return;
        }
        document.addEventListener(
            'visibilitychange',
            this.onVisibilityChanged.createDelegate(this)
        );
    },

    onVisibilityChanged: function () {
        if (this.hidden()) {
            if (this.running) {
                clearTimeout(this.running);
                this.running = undefined;
            }
            this.keepSessionAlive();
            return;
        }
        if (this.heartbeat) {
            clearTimeout(this.heartbeat);
            this.heartbeat = undefined;
        }
        // Back in front. The list is as old as the time away, so it is asked
        // for now rather than at the next tick — and what comes back is the
        // difference since the last answer, however long ago that was.
        this.update();
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

        // The epoch the server last gave us. It answers with the difference
        // since that one, or with the whole list and a new epoch when it
        // cannot — which is what happens after a reload, after a dropped
        // answer, and whenever the question changes. Asking a different
        // question is one of those, so a filter change starts again from
        // nothing rather than applying a difference to a list that was about
        // to be replaced.
        if (!Ext.areObjectsEqual(this.filters, this.oldFilters)) {
            this.epoch = 0;
        }

        // Only the keys the columns on screen are drawn from: what nobody is
        // looking at is not worth building, sending or parsing.
        var keys = Deluge.Keys.forGrid(deluge.torrents);
        deluge.client.web.update_ui(keys, filters, this.epoch || 0, {
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
        // And the answer that did not arrive may have been a difference the
        // server has already counted as sent, so the next poll asks for the
        // whole list.
        this.epoch = 0;
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
        this.epoch = data['epoch'] || 0;

        // A difference is applied to the list we already have, and what comes
        // out is the whole list again — so everything below this line, and
        // everything the grid does with it, is unchanged.
        // A difference with nothing to apply it to would draw whatever moved
        // and call it the library. It should not be possible — the server only
        // sends one when it knows what we hold — so this asks again rather
        // than trying to make sense of it.
        if (data['delta'] && !deluge.torrents.lastTorrents) {
            this.epoch = 0;
            this.update();
            return;
        }

        var torrents = data['delta']
            ? deluge.torrents.merge(data['torrents'], data['removed'])
            : data['torrents'];

        if (Ext.areObjectsEqual(this.filters, this.oldFilters)) {
            deluge.torrents.update(torrents);
        } else {
            deluge.torrents.update(torrents, true);
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
