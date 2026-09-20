/**
 * Deluge.preferences.Rss.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Feeds, and the rules that pull torrents out of them.
 *
 * The watched folders page is the other half of this: there, something else
 * decides what to download and drops a file in a directory. Here the decision
 * is a line in a rule, and the feed is a page somebody else serves.
 *
 * Two grids rather than a wizard. A feed is a name and an address; a rule is a
 * pattern and what to do with what it matches. Both are short enough to edit
 * in place, and seeing all of them at once is how you spot the rule that is
 * quietly taking everything.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Rss
 * @extends Ext.form.FormPanel
 */
Deluge.preferences.Rss = Ext.extend(Ext.form.FormPanel, {
    constructor: function (config) {
        config = Ext.apply(
            {
                border: false,
                title: _('Feeds'),
                header: false,
                layout: 'form',
                autoScroll: true,
            },
            config
        );
        Deluge.preferences.Rss.superclass.constructor.call(this, config);
    },

    initComponent: function () {
        Deluge.preferences.Rss.superclass.initComponent.call(this);

        this.enabled = this.add({
            xtype: 'checkbox',
            name: 'rss_enabled',
            hideLabel: true,
            boxLabel: _('Read these feeds and act on them'),
        });

        this.add({
            xtype: 'label',
            text: _(
                'A rule adds torrents by itself, so it is written to be boring: a rule with no pattern takes nothing, the first look at a feed downloads nothing and only notes what is already in it, and an item is acted on once.'
            ),
            style: 'display: block; margin: 2px 0 8px 0; opacity: 0.72;',
        });

        this.interval = this.add({
            xtype: 'spinnerfield',
            fieldLabel: _('Read every (minutes):'),
            labelSeparator: '',
            name: 'rss_interval',
            width: 80,
            decimalPrecision: 0,
            minValue: 1,
            maxValue: 1440,
            incrementValue: 5,
            style: 'margin-bottom: 8px;',
        });

        // ------------------------------------------------------------ feeds
        this.add({
            xtype: 'label',
            text: _('Feeds'),
            style: 'display: block; font-weight: bold; margin: 6px 0 2px 0;',
        });

        this.feedStore = new Ext.data.ArrayStore({
            fields: [
                { name: 'enabled', type: 'bool' },
                { name: 'name', type: 'string' },
                { name: 'url', type: 'string' },
            ],
        });

        this.feeds = this.add({
            xtype: 'editorgrid',
            store: this.feedStore,
            height: 120,
            anchor: '100%',
            clicksToEdit: 1,
            style: 'margin: 4px 0 10px 0;',
            selModel: new Ext.grid.RowSelectionModel({ singleSelect: true }),
            columns: [
                {
                    header: _('On'),
                    dataIndex: 'enabled',
                    width: 36,
                    renderer: this.renderTick,
                    editor: { xtype: 'checkbox' },
                },
                {
                    header: _('Name'),
                    dataIndex: 'name',
                    width: 120,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Address'),
                    dataIndex: 'url',
                    width: 340,
                    editor: { xtype: 'textfield' },
                },
            ],
            bbar: [
                {
                    text: _('Add'),
                    iconCls: 'icon-add',
                    handler: this.onAddFeed,
                    scope: this,
                },
                {
                    text: _('Remove'),
                    iconCls: 'icon-remove',
                    handler: this.onRemoveFeed,
                    scope: this,
                },
                '->',
                {
                    // Asked before a rule is left running for a month: what
                    // does this feed actually offer, and which rule takes it?
                    text: _('Test the selected feed'),
                    handler: this.onTestFeed,
                    scope: this,
                },
            ],
        });

        // ------------------------------------------------------------ rules
        this.add({
            xtype: 'label',
            text: _('Rules'),
            style: 'display: block; font-weight: bold; margin: 6px 0 2px 0;',
        });

        this.ruleStore = new Ext.data.ArrayStore({
            fields: [
                { name: 'enabled', type: 'bool' },
                { name: 'name', type: 'string' },
                { name: 'feed', type: 'string' },
                { name: 'contains', type: 'string' },
                { name: 'excludes', type: 'string' },
                { name: 'label', type: 'string' },
                { name: 'save_path', type: 'string' },
                { name: 'paused', type: 'bool' },
            ],
        });

        this.rules = this.add({
            xtype: 'editorgrid',
            store: this.ruleStore,
            height: 180,
            anchor: '100%',
            clicksToEdit: 1,
            style: 'margin: 4px 0 8px 0;',
            selModel: new Ext.grid.RowSelectionModel({ singleSelect: true }),
            columns: [
                {
                    header: _('On'),
                    dataIndex: 'enabled',
                    width: 36,
                    renderer: this.renderTick,
                    editor: { xtype: 'checkbox' },
                },
                {
                    header: _('Name'),
                    dataIndex: 'name',
                    width: 90,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Feed'),
                    dataIndex: 'feed',
                    width: 80,
                    renderer: function (value) {
                        return value
                            ? Ext.util.Format.htmlEncode(value)
                            : '<span style="opacity:0.6">' + _('any') + '</span>';
                    },
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Title contains'),
                    dataIndex: 'contains',
                    width: 150,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('But not'),
                    dataIndex: 'excludes',
                    width: 100,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Label'),
                    dataIndex: 'label',
                    width: 80,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Download to'),
                    dataIndex: 'save_path',
                    width: 120,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Paused'),
                    dataIndex: 'paused',
                    width: 56,
                    renderer: this.renderTick,
                    editor: { xtype: 'checkbox' },
                },
            ],
            bbar: [
                {
                    text: _('Add'),
                    iconCls: 'icon-add',
                    handler: this.onAddRule,
                    scope: this,
                },
                {
                    text: _('Remove'),
                    iconCls: 'icon-remove',
                    handler: this.onRemoveRule,
                    scope: this,
                },
            ],
        });

        this.add({
            xtype: 'label',
            text: _(
                'A pattern is what you would type while looking at the titles in a feed: * stands for anything, case does not matter, and the parts have to appear in the order you wrote them. Winter.Harbour*1080p takes the 1080p ones and leaves the rest. A rule that names no feed applies to all of them, and the first rule that wants an item is the one that gets it.'
            ),
            style: 'display: block; margin: 2px 0 0 0; opacity: 0.72;',
        });

        this.on('show', this.onPageShow, this);
    },

    renderTick: function (value) {
        return value ? '&#10003;' : '';
    },

    // Not `onAdd`/`onRemove`: Ext.Container calls methods of those names on
    // every child it gains, which would run these against a store that does
    // not exist yet.
    onAddFeed: function () {
        this.feedStore.add(
            new this.feedStore.recordType({ enabled: true, name: '', url: '' })
        );
    },

    onRemoveFeed: function () {
        var selected = this.feeds.getSelectionModel().getSelected();
        if (selected) this.feedStore.remove(selected);
    },

    onAddRule: function () {
        this.ruleStore.add(
            new this.ruleStore.recordType({
                enabled: true,
                name: '',
                feed: '',
                contains: '',
                excludes: '',
                label: '',
                save_path: '',
                paused: false,
            })
        );
    },

    onRemoveRule: function () {
        var selected = this.rules.getSelectionModel().getSelected();
        if (selected) this.ruleStore.remove(selected);
    },

    /**
     * Shows what a feed offers and what the rules would do with it.
     *
     * The rules as the daemon has them, not as the grid has them: the answer
     * is about what will happen, and what will happen is what was saved. A
     * rule edited and not yet applied says so rather than being guessed at.
     */
    onTestFeed: function () {
        var selected = this.feeds.getSelectionModel().getSelected();
        if (!selected) {
            Ext.MessageBox.show({
                title: _('Test a feed'),
                msg: _('Pick a feed in the list first.'),
                buttons: Ext.MessageBox.OK,
                icon: Ext.MessageBox.INFO,
            });
            return;
        }

        var url = String(selected.get('url') || '').trim();
        if (!url) {
            Ext.MessageBox.show({
                title: _('Test a feed'),
                msg: _('That feed has no address yet.'),
                buttons: Ext.MessageBox.OK,
                icon: Ext.MessageBox.INFO,
            });
            return;
        }

        var dirty = this.feedStore.getModifiedRecords().length > 0
            || this.ruleStore.getModifiedRecords().length > 0;

        Ext.MessageBox.wait(_('Reading the feed...'), _('Test a feed'));
        deluge.client.redeluge.test_feed(url, String(selected.get('name') || ''), {
            success: function (answer) {
                Ext.MessageBox.hide();
                this.showFeedResult(answer, dirty);
            },
            failure: function (error) {
                Ext.MessageBox.hide();
                Ext.MessageBox.show({
                    title: _('Test a feed'),
                    msg: (error && error.error && error.error.message)
                        || _('That feed could not be read.'),
                    buttons: Ext.MessageBox.OK,
                    icon: Ext.MessageBox.ERROR,
                });
            },
            scope: this,
        });
    },

    showFeedResult: function (answer, dirty) {
        var items = (answer && answer['items']) || [];
        if (!this.testWindow) {
            this.testStore = new Ext.data.ArrayStore({
                fields: ['taken', 'title', 'rule', 'label'],
            });
            this.testWindow = new Ext.Window({
                title: _('What this feed offers'),
                width: 620,
                height: 380,
                layout: 'fit',
                closeAction: 'hide',
                constrainHeader: true,
                plain: true,
                items: [
                    {
                        xtype: 'grid',
                        store: this.testStore,
                        viewConfig: { forceFit: true },
                        columns: [
                            {
                                header: _('Taken'),
                                dataIndex: 'taken',
                                width: 50,
                                renderer: function (value) {
                                    return value ? '&#10003;' : '';
                                },
                            },
                            {
                                header: _('Title'),
                                dataIndex: 'title',
                                width: 320,
                                renderer: Ext.util.Format.htmlEncode,
                            },
                            {
                                header: _('By rule'),
                                dataIndex: 'rule',
                                width: 100,
                                renderer: Ext.util.Format.htmlEncode,
                            },
                            {
                                header: _('Label'),
                                dataIndex: 'label',
                                width: 80,
                                renderer: Ext.util.Format.htmlEncode,
                            },
                        ],
                    },
                ],
                bbar: [{ xtype: 'tbtext', text: '' }],
                buttons: [
                    {
                        text: _('Close'),
                        handler: function () {
                            this.testWindow.hide();
                        },
                        scope: this,
                    },
                ],
            });
        }

        this.testStore.loadData(
            items.map(function (item) {
                return [item['taken'] === true, item['title'] || '', item['rule'] || '', item['label'] || ''];
            })
        );

        var taken = items.filter(function (item) { return item['taken'] === true; }).length;
        var note = String.format(
            _('{0} items, {1} of which the rules would take. Nothing was added.'),
            items.length,
            taken
        );
        if (dirty) {
            note += ' ' + _('The rules used are the ones already saved: apply your changes and test again to try the new ones.');
        }
        this.testWindow.getBottomToolbar().items.items[0].setText(note);
        this.testWindow.show();
    },

    onPageShow: function () {
        if (this.loaded) return;
        this.loaded = true;
        deluge.client.core.get_config({
            success: this.onGotConfig,
            scope: this,
        });
    },

    onGotConfig: function (config) {
        var settings = (config && config['rss']) || {};
        this.enabled.setValue(settings['enabled'] === true);
        this.interval.setValue(Deluge.number(settings['interval'], 30));

        this.feedStore.loadData(
            (settings['feeds'] || []).map(function (feed) {
                return [
                    feed['enabled'] !== false,
                    feed['name'] || '',
                    feed['url'] || '',
                ];
            })
        );

        this.ruleStore.loadData(
            (settings['rules'] || []).map(function (rule) {
                return [
                    rule['enabled'] !== false,
                    rule['name'] || '',
                    rule['feed'] || '',
                    rule['contains'] || '',
                    rule['excludes'] || '',
                    rule['label'] || '',
                    rule['save_path'] || '',
                    rule['paused'] === true,
                ];
            })
        );
    },

    onApply: function () {
        // Nothing read means nothing of this page's to write: Preferences
        // applies every page on OK, and an unread page holds blanks.
        if (!this.loaded) return;

        var feeds = [];
        this.feedStore.each(function (record) {
            feeds.push({
                enabled: record.get('enabled') === true,
                name: String(record.get('name') || '').trim(),
                url: String(record.get('url') || '').trim(),
            });
        });

        var rules = [];
        this.ruleStore.each(function (record) {
            rules.push({
                enabled: record.get('enabled') === true,
                name: String(record.get('name') || '').trim(),
                feed: String(record.get('feed') || '').trim(),
                contains: String(record.get('contains') || '').trim(),
                excludes: String(record.get('excludes') || '').trim(),
                label: String(record.get('label') || '').trim(),
                save_path: String(record.get('save_path') || '').trim(),
                paused: record.get('paused') === true,
            });
        });

        deluge.client.core.set_config({
            rss: {
                enabled: this.enabled.getValue() === true,
                interval: Deluge.number(this.interval.getValue(), 30),
                feeds: feeds,
                rules: rules,
            },
        });
    },

    onOk: function () {
        this.onApply();
    },
});
